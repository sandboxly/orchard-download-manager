//! The download engine: one controller thread per running download, and a
//! thread per connection. A file the server serves in ranges is split into
//! segments fetched in parallel; when a connection finishes its segment it
//! takes over half of the largest one still left, so all connections keep
//! working until the end. Bytes go straight to their place in
//! "<name>.part", which is renamed when complete. Pausing keeps the
//! segments, so the download resumes where it stopped, also after a restart
//! of the app.
//!
//! The controller reports through a callback (`Event`); progress is read
//! from [`Shared`] by whoever wants it.

pub mod http;
pub mod limiter;

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use self::http::{Headers, Probe, Validator};
use self::limiter::Limiter;

/// A segment's end when the size is unknown.
pub const UNKNOWN_END: u64 = u64::MAX;

/// Bytes `[start, end)` of the file, of which `[start, pos)` are on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub pos: u64,
}

impl Segment {
    pub fn new(start: u64, end: u64) -> Segment {
        Segment { start, end, pos: start }
    }

    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.pos)
    }

    pub fn done(&self) -> bool {
        self.pos >= self.end
    }

    pub fn received(&self) -> u64 {
        self.pos - self.start
    }
}

/// What to download and how.
#[derive(Clone, Debug)]
pub struct Spec {
    pub url: String,
    /// Where the file goes, if decided (else the app is asked after the probe).
    pub target: Option<PathBuf>,
    /// From an earlier run: the size, segments and validator to resume with.
    pub size: Option<u64>,
    pub segments: Vec<Segment>,
    pub validator: Validator,
    pub headers: Headers,
    pub connections: usize,
    /// Never split below this.
    pub min_segment: u64,
    pub retries: u32,
    pub retry_delay: Duration,
    /// A connection that delivers nothing for this long is replaced.
    pub stall: Duration,
    /// An existing file at the target is replaced (else the name gets a number).
    pub overwrite: bool,
}

impl Spec {
    pub fn new(url: impl Into<String>) -> Spec {
        Spec {
            url: url.into(),
            target: None,
            size: None,
            segments: Vec::new(),
            validator: Validator::default(),
            headers: Vec::new(),
            connections: 8,
            min_segment: 1024 * 1024,
            retries: 5,
            retry_delay: Duration::from_secs(5),
            stall: Duration::from_secs(30),
            overwrite: false,
        }
    }
}

/// What the controller reports.
#[derive(Debug)]
pub enum Event {
    /// The server answered; reply with where to save (`None` cancels).
    Probed { probe: Probe, reply: Sender<Option<PathBuf>> },
    /// Connections are running (`restarted`: why progress from before was dropped).
    Running { probe: Probe, target: PathBuf, restarted: Option<String> },
    /// Stopped on request; the segments in `Shared` are where to resume.
    Paused,
    Done(PathBuf),
    Failed(String),
}

pub type Reporter = Arc<dyn Fn(Event) + Send + Sync>;

/// One segment and who is working on it.
#[derive(Clone, Debug)]
struct Slot {
    seg: Segment,
    /// The worker fetching it (0: none).
    owner: u64,
    /// The last time its worker made progress.
    touched: Instant,
    failures: u32,
    not_before: Option<Instant>,
}

/// State shared between the controller, its connections and the app.
pub struct Shared {
    stop: AtomicBool,
    /// Bytes on disk.
    pub received: AtomicU64,
    /// Connections running now.
    pub connections: AtomicUsize,
    /// This download's own speed limit.
    pub limit: Limiter,
    slots: Mutex<Vec<Slot>>,
    /// The last connection problem (shown while retrying).
    pub problem: Mutex<Option<String>>,
}

impl Shared {
    pub fn new(limit: u64) -> Arc<Shared> {
        Arc::new(Shared {
            stop: AtomicBool::new(false),
            received: AtomicU64::new(0),
            connections: AtomicUsize::new(0),
            limit: Limiter::new(limit),
            slots: Mutex::new(Vec::new()),
            problem: Mutex::new(None),
        })
    }

    /// Ask the download to stop (it reports `Paused`).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    /// The segments now, in file order.
    pub fn segments(&self) -> Vec<Segment> {
        let mut v: Vec<Segment> = self.slots.lock().unwrap().iter().map(|s| s.seg).collect();
        v.sort_by_key(|s| s.start);
        v
    }

    /// Set the segments, `active` ones as being fetched (screenshots, tests).
    pub fn set_segments(&self, segs: &[Segment], active: usize) {
        let now = Instant::now();
        *self.slots.lock().unwrap() = segs
            .iter()
            .enumerate()
            .map(|(i, &seg)| Slot { seg, owner: if i < active { i as u64 + 1 } else { 0 }, touched: now, failures: 0, not_before: None })
            .collect();
        self.received.store(segs.iter().map(Segment::received).sum(), Ordering::Relaxed);
        self.connections.store(active, Ordering::Relaxed);
    }

    /// The segments being fetched now.
    pub fn active_segments(&self) -> Vec<Segment> {
        self.slots.lock().unwrap().iter().filter(|s| s.owner != 0).map(|s| s.seg).collect()
    }
}

/// Start downloading on a new thread.
pub fn start(spec: Spec, shared: Arc<Shared>, global: Arc<Limiter>, agent: ureq::Agent, report: Reporter) {
    let name = format!("download-{}", http::url_filename(&spec.url));
    let r = report.clone();
    let spawned = thread::Builder::new().name(name).spawn(move || {
        let event = match run(spec, &shared, global, agent, &report) {
            Ok(Outcome::Done(p)) => Event::Done(p),
            Ok(Outcome::Paused) => Event::Paused,
            Err(e) => Event::Failed(e),
        };
        shared.connections.store(0, Ordering::Relaxed);
        report(event);
    });
    if let Err(e) = spawned {
        r(Event::Failed(format!("couldn't start a thread ({e})")));
    }
}

enum Outcome {
    Done(PathBuf),
    Paused,
}

/// "<name>.part" next to the target.
pub fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    target.with_file_name(name)
}

/// `path`, or "name (1).ext", "name (2).ext"… : the first that neither
/// exists (nor its part file) nor is `taken`.
pub fn unique_path(path: &Path, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let free = |p: &Path| !p.exists() && !part_path(p).exists() && !taken(p);
    if free(path) {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    // "archive.tar.gz" → "archive (1).tar.gz".
    let (stem, ext) = match stem.strip_suffix(".tar") {
        Some(s) => (s.to_string(), format!(".tar{ext}")),
        None => (stem, ext),
    };
    (1..)
        .map(|i| path.with_file_name(format!("{stem} ({i}){ext}")))
        .find(|p| free(p))
        .expect("an unused name")
}

/// Initial segments: equal parts, one per connection, none below `min`.
pub fn initial_segments(size: Option<u64>, resumable: bool, connections: usize, min: u64) -> Vec<Segment> {
    match size {
        Some(0) => vec![Segment::new(0, 0)],
        Some(n) if resumable => {
            let parts = (n / min.max(1)).clamp(1, connections.max(1) as u64);
            let step = n / parts;
            (0..parts).map(|i| Segment::new(i * step, if i + 1 == parts { n } else { (i + 1) * step })).collect()
        }
        Some(n) => vec![Segment::new(0, n)],
        None => vec![Segment::new(0, UNKNOWN_END)],
    }
}

/// Split the largest segment still being fetched, if its rest is at least
/// two minimum segments: the new one takes the second half. Returns its index.
fn split_largest(slots: &mut Vec<Slot>, min: u64) -> Option<usize> {
    let (i, rem) = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.owner != 0 && s.seg.end != UNKNOWN_END)
        .map(|(i, s)| (i, s.seg.remaining()))
        .max_by_key(|(_, r)| *r)?;
    if rem < 2 * min.max(1) {
        return None;
    }
    let s = &mut slots[i].seg;
    let mid = s.pos + rem / 2;
    let new = Segment::new(mid, s.end);
    s.end = mid;
    slots.push(Slot { seg: new, owner: 0, touched: Instant::now(), failures: 0, not_before: None });
    Some(slots.len() - 1)
}

/// Retry `f` with the spec's delay; `None` once stopped.
fn with_retries<T>(spec: &Spec, shared: &Shared, mut f: impl FnMut() -> Result<T, String>) -> Option<Result<T, String>> {
    let mut tries = 0;
    loop {
        match f() {
            Ok(v) => return Some(Ok(v)),
            Err(e) if tries >= spec.retries || is_final(&e) => return Some(Err(e)),
            Err(e) => {
                *shared.problem.lock().unwrap() = Some(e);
                tries += 1;
            }
        }
        let until = Instant::now() + spec.retry_delay * tries;
        while Instant::now() < until {
            if shared.stopping() {
                return None;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Errors trying again won't fix.
fn is_final(e: &str) -> bool {
    e.contains("(404)") || e.contains("(410)") || e.contains("(401)") || e.contains("(403)") || e.contains("valid link")
}

fn run(spec: Spec, shared: &Arc<Shared>, global: Arc<Limiter>, agent: ureq::Agent, report: &Reporter) -> Result<Outcome, String> {
    let Some(probe) = with_retries(&spec, shared, || http::probe(&agent, &spec.url, &spec.headers)) else {
        return Ok(Outcome::Paused);
    };
    let probe = probe?;
    *shared.problem.lock().unwrap() = None;
    let had_progress = spec.segments.iter().any(|s| s.received() > 0);
    let mut resume = !spec.segments.is_empty() && probe.resumable && spec.size == probe.size && !spec.validator.changed(&probe.validator);
    let mut restarted = (had_progress && !resume).then(|| {
        if !probe.resumable {
            "the server can't resume downloads".to_string()
        } else {
            "the file on the server changed".to_string()
        }
    });

    let target = match &spec.target {
        Some(t) => t.clone(),
        None => {
            let (tx, rx) = mpsc::channel();
            report(Event::Probed { probe: probe.clone(), reply: tx });
            match rx.recv() {
                Ok(Some(p)) => p,
                _ => return Ok(Outcome::Paused),
            }
        }
    };
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("can't create the folder {}: {}", dir.display(), crate::format::io_reason(&e)))?;
    }
    let part = part_path(&target);
    if resume && !part.exists() {
        resume = false;
        restarted = had_progress.then(|| "the partial file is gone".to_string());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&part)
        .map_err(|e| format!("can't write {}: {}", part.display(), crate::format::io_reason(&e)))?;
    if !resume {
        file.set_len(0).map_err(|e| crate::format::io_reason(&e))?;
        if let Some(n) = probe.size {
            // Reserve the space now: a full disk shows at once, not at 97 %.
            file.set_len(n).map_err(|e| format!("no room for {}: {}", crate::format::size(n), crate::format::io_reason(&e)))?;
        }
    }
    drop(file);
    let connections = if probe.resumable { spec.connections.max(1) } else { 1 };
    let segments = if resume { spec.segments.clone() } else { initial_segments(probe.size, probe.resumable, connections, spec.min_segment) };
    shared.received.store(segments.iter().map(Segment::received).sum(), Ordering::Relaxed);
    *shared.slots.lock().unwrap() =
        segments.into_iter().map(|seg| Slot { seg, owner: 0, touched: Instant::now(), failures: 0, not_before: None }).collect();
    report(Event::Running { probe: probe.clone(), target: target.clone(), restarted });

    let ctx = Arc::new(WorkerCtx {
        url: probe.final_url.clone(),
        headers: spec.headers.clone(),
        part: part.clone(),
        resumable: probe.resumable,
        agent,
        global,
        shared: shared.clone(),
    });
    let (tx, rx) = mpsc::channel::<(u64, Result<(), WorkerError>)>();
    let mut live: HashSet<u64> = HashSet::new();
    let mut next_owner = 1u64;
    let failure: Option<String> = loop {
        if shared.stopping() {
            break None;
        }
        {
            let mut slots = shared.slots.lock().unwrap();
            if slots.iter().all(|s| s.seg.done()) {
                break None;
            }
            let now = Instant::now();
            // Replace stalled connections.
            for s in slots.iter_mut().filter(|s| s.owner != 0 && now.duration_since(s.touched) > spec.stall) {
                live.remove(&s.owner);
                s.owner = 0;
                s.failures += 1;
                *shared.problem.lock().unwrap() = Some("a connection stalled".into());
            }
            // Idle segments first, then splits.
            for (i, s) in slots.iter_mut().enumerate() {
                if live.len() >= connections {
                    break;
                }
                if s.owner == 0 && !s.seg.done() && s.not_before.is_none_or(|t| now >= t) {
                    if !probe.resumable && s.seg.pos > s.seg.start {
                        // Can't continue in the middle: from the start again.
                        shared.received.fetch_sub(s.seg.received(), Ordering::Relaxed);
                        s.seg.pos = s.seg.start;
                    }
                    s.owner = next_owner;
                    s.touched = now;
                    s.not_before = None;
                    spawn_worker(&ctx, i, next_owner, tx.clone());
                    live.insert(next_owner);
                    next_owner += 1;
                }
            }
            let waiting = slots.iter().any(|s| s.owner == 0 && !s.seg.done());
            while probe.resumable && !waiting && live.len() < connections {
                let Some(i) = split_largest(&mut slots, spec.min_segment) else { break };
                slots[i].owner = next_owner;
                spawn_worker(&ctx, i, next_owner, tx.clone());
                live.insert(next_owner);
                next_owner += 1;
            }
            shared.connections.store(live.len(), Ordering::Relaxed);
        }
        let Ok((owner, result)) = rx.recv_timeout(Duration::from_millis(200)) else { continue };
        if !live.remove(&owner) {
            continue; // replaced after stalling
        }
        let mut slots = shared.slots.lock().unwrap();
        let Some(s) = slots.iter_mut().find(|s| s.owner == owner) else { continue };
        s.owner = 0;
        match result {
            Ok(()) => {
                s.failures = 0;
                *shared.problem.lock().unwrap() = None;
            }
            Err(WorkerError::Stopped) => {}
            Err(WorkerError::Fatal(e)) => break Some(e),
            Err(WorkerError::Retry(e)) => {
                s.failures += 1;
                if s.failures > spec.retries || is_final(&e) {
                    break Some(e);
                }
                s.not_before = Some(Instant::now() + spec.retry_delay * s.failures);
                *shared.problem.lock().unwrap() = Some(e);
            }
        }
    };

    // Let the connections finish their last write, then hand back.
    let paused = failure.is_none() && shared.stopping();
    if paused || failure.is_some() {
        shared.stop();
        let until = Instant::now() + Duration::from_secs(3);
        while !live.is_empty() && Instant::now() < until {
            if let Ok((owner, _)) = rx.recv_timeout(Duration::from_millis(100)) {
                live.remove(&owner);
            }
        }
        for s in shared.slots.lock().unwrap().iter_mut() {
            s.owner = 0;
        }
        if let Some(e) = failure {
            return Err(e);
        }
        return Ok(Outcome::Paused);
    }
    finish(&part, &target, &probe, shared, spec.overwrite).map(Outcome::Done)
}

/// The part file is complete: check its size and give it its name.
fn finish(part: &Path, target: &Path, probe: &Probe, shared: &Shared, overwrite: bool) -> Result<PathBuf, String> {
    let got = shared.received();
    if let Some(n) = probe.size
        && got != n
    {
        return Err(format!("the server sent {} of {}", crate::format::size(got), crate::format::size(n)));
    }
    if let Ok(f) = File::open(part) {
        let _ = f.sync_all();
    }
    let dest = if target.exists() && !overwrite { unique_path(target, |_| false) } else { target.to_path_buf() };
    if overwrite && dest.exists() {
        let _ = fs::remove_file(&dest);
    }
    fs::rename(part, &dest).map_err(|e| format!("can't rename the finished file: {}", crate::format::io_reason(&e)))?;
    Ok(dest)
}

struct WorkerCtx {
    url: String,
    headers: Headers,
    part: PathBuf,
    resumable: bool,
    agent: ureq::Agent,
    global: Arc<Limiter>,
    shared: Arc<Shared>,
}

#[derive(Debug)]
enum WorkerError {
    /// Stopped, or the segment went to someone else.
    Stopped,
    /// A network problem: try again.
    Retry(String),
    /// A disk problem: retrying won't help.
    Fatal(String),
}

fn spawn_worker(ctx: &Arc<WorkerCtx>, slot: usize, owner: u64, tx: Sender<(u64, Result<(), WorkerError>)>) {
    let ctx = ctx.clone();
    let spawned = thread::Builder::new().name(format!("connection-{owner}")).spawn(move || {
        let r = fetch(&ctx, slot, owner);
        let _ = tx.send((owner, r));
    });
    if spawned.is_err() {
        log::warn!("couldn't start a connection thread");
    }
}

/// Fetch one segment until it's done, shrunk away, stopped or broken.
fn fetch(ctx: &WorkerCtx, slot: usize, owner: u64) -> Result<(), WorkerError> {
    let shared = &ctx.shared;
    let (mut pos, end) = {
        let slots = shared.slots.lock().unwrap();
        let s = slots.get(slot).ok_or(WorkerError::Stopped)?;
        (s.seg.pos, s.seg.end)
    };
    let range = if ctx.resumable { Some((pos, (end != UNKNOWN_END).then(|| end - 1))) } else { None };
    let resp = http::get(&ctx.agent, &ctx.url, range, &ctx.headers).map_err(WorkerError::Retry)?;
    let status = resp.status().as_u16();
    match status {
        206 => {
            let start = resp.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(http::parse_content_range).map(|r| r.0);
            if start != Some(pos) {
                return Err(WorkerError::Retry("the server sent the wrong part of the file".into()));
            }
        }
        200 if pos == 0 || !ctx.resumable => {}
        200 => return Err(WorkerError::Retry("the server stopped sending parts of the file".into())),
        s => return Err(WorkerError::Retry(http::status_reason(s))),
    }
    let file = OpenOptions::new().write(true).open(&ctx.part).map_err(|e| WorkerError::Fatal(crate::format::io_reason(&e)))?;
    let mut body = resp.into_body().into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if shared.stopping() {
            return Err(WorkerError::Stopped);
        }
        let want = shared.limit.take(buf.len(), &shared.stop);
        let want = ctx.global.take(want, &shared.stop);
        if want == 0 {
            return Err(WorkerError::Stopped);
        }
        let n = match body.read(&mut buf[..want]) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(WorkerError::Retry(http::io_net_reason(&e))),
        };
        // How much of it is still ours (the segment may have been split).
        let allowed = {
            let mut slots = shared.slots.lock().unwrap();
            let s = &mut slots[slot];
            if s.owner != owner {
                return Err(WorkerError::Stopped);
            }
            if n == 0 {
                if s.seg.end == UNKNOWN_END {
                    s.seg.end = s.seg.pos;
                    return Ok(());
                }
                return if s.seg.done() { Ok(()) } else { Err(WorkerError::Retry("the connection closed early".into())) };
            }
            s.touched = Instant::now();
            (n as u64).min(s.seg.end - pos) as usize
        };
        write_at(&file, &buf[..allowed], pos).map_err(|e| WorkerError::Fatal(format!("can't write the file: {}", crate::format::io_reason(&e))))?;
        pos += allowed as u64;
        let mut slots = shared.slots.lock().unwrap();
        let s = &mut slots[slot];
        if s.owner != owner {
            // Replaced meanwhile: the new connection counts these bytes.
            return Err(WorkerError::Stopped);
        }
        // Split meanwhile: bytes past the new end are the new segment's (its
        // connection writes the same bytes there again).
        let before = s.seg.pos;
        s.seg.pos = pos.min(s.seg.end).max(before);
        shared.received.fetch_add(s.seg.pos - before, Ordering::Relaxed);
        if s.seg.done() {
            return Ok(());
        }
    }
}

fn write_at(f: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        f.write_all_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            let n = f.seek_write(&buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            done += n;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests;
