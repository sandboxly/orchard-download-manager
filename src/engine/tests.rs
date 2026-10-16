//! The engine against a small local HTTP server: ranges, splitting,
//! pausing and resuming, servers without ranges, broken connections.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::*;

/// How the test server behaves.
#[derive(Clone, Default)]
pub struct ServerOpts {
    pub ranges: bool,
    /// Bytes per second per connection (0: as fast as it goes).
    pub rate: usize,
    /// Close every n-th response after half of its bytes (0: never).
    pub break_every: usize,
    pub etag: Option<String>,
    pub disposition: Option<String>,
    /// Answer every request with this status.
    pub status: Option<u16>,
    /// No Content-Length on full responses.
    pub chunked_unknown: bool,
    /// Answer 403 to requests without exactly this Cookie header.
    pub cookie: Option<String>,
}

pub struct Server {
    pub url: String,
    pub requests: Arc<AtomicUsize>,
    /// The most connections served at once.
    pub peak: Arc<AtomicUsize>,
    pub opts: Arc<Mutex<ServerOpts>>,
}

pub fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

pub fn serve(content: Vec<u8>, opts: ServerOpts) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/files/test%20file.bin", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let current = Arc::new(AtomicUsize::new(0));
    let data = Arc::new(Mutex::new(content));
    let opts = Arc::new(Mutex::new(opts));
    let (r, p, d, o) = (requests.clone(), peak.clone(), data.clone(), opts.clone());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let n = r.fetch_add(1, Ordering::SeqCst) + 1;
            let (p, c, d, o) = (p.clone(), current.clone(), d.clone(), o.lock().unwrap().clone());
            thread::spawn(move || {
                let now = c.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(now, Ordering::SeqCst);
                let data = d.lock().unwrap().clone();
                let _ = handle(stream, &data, &o, n);
                c.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Server { url, requests, peak, opts }
}

fn handle(mut s: TcpStream, data: &[u8], o: &ServerOpts, n: usize) -> std::io::Result<()> {
    let mut reader = BufReader::new(s.try_clone()?);
    let mut range = None;
    let mut cookie = None;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        if h.trim().is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
            let (a, b) = v.trim().split_once('-').unwrap();
            let a: usize = a.parse().unwrap();
            let last = data.len().saturating_sub(1);
            let b: usize = if b.is_empty() { last } else { b.parse().unwrap() };
            range = Some((a, b.min(last)));
        }
        if let Some((k, v)) = h.split_once(':')
            && k.eq_ignore_ascii_case("cookie")
        {
            cookie = Some(v.trim().to_string());
        }
    }
    let status = match &o.cookie {
        Some(want) if cookie.as_ref() != Some(want) => Some(403),
        _ => o.status,
    };
    if let Some(st) = status {
        write!(s, "HTTP/1.1 {st} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }
    let mut head = String::new();
    let partial = range.filter(|_| o.ranges && !data.is_empty());
    let body = match partial {
        Some((a, b)) => {
            head.push_str(&format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\n", data.len()));
            &data[a..=b]
        }
        None => {
            head.push_str("HTTP/1.1 200 OK\r\n");
            data
        }
    };
    if !(o.chunked_unknown && partial.is_none()) {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    if let Some(e) = &o.etag {
        head.push_str(&format!("ETag: {e}\r\n"));
    }
    if let Some(d) = &o.disposition {
        head.push_str(&format!("Content-Disposition: {d}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    s.write_all(head.as_bytes())?;
    let cut = if o.break_every > 0 && n.is_multiple_of(o.break_every) { body.len() / 2 } else { body.len() };
    let chunk = if o.rate > 0 { (o.rate / 20).max(1) } else { 64 * 1024 };
    for part in body[..cut].chunks(chunk) {
        s.write_all(part)?;
        if o.rate > 0 {
            thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}

/// Run a download to its end; returns the events in order.
fn run_to_end(spec: Spec, shared: Arc<Shared>, dir: &Path) -> Vec<String> {
    let (tx, rx) = mpsc::channel();
    let dir = dir.to_path_buf();
    let report: Reporter = Arc::new(move |e| {
        let text = match &e {
            Event::Probed { probe, reply } => {
                let _ = reply.send(Some((dir.join(&probe.name), false)));
                format!("probed {} {:?} {}", probe.name, probe.size, probe.resumable)
            }
            Event::Running { restarted, .. } => format!("running {restarted:?}"),
            Event::Paused => "paused".into(),
            Event::Done(p) => format!("done {}", p.file_name().unwrap().to_string_lossy()),
            Event::Failed(e) => format!("failed {e}"),
        };
        let end = matches!(e, Event::Paused | Event::Done(_) | Event::Failed(_));
        let _ = tx.send((text, end));
    });
    start(spec, shared, Arc::new(Limiter::new(0)), test_agent(), report);
    let mut out = Vec::new();
    loop {
        let (t, end) = rx.recv_timeout(Duration::from_secs(30)).expect("the download ends");
        out.push(t);
        if end {
            return out;
        }
    }
}

fn test_agent() -> ureq::Agent {
    http::agent(&http::NetConfig { proxy: None, timeout: Duration::from_secs(5), ..Default::default() }).unwrap()
}

fn spec(url: &str) -> Spec {
    Spec { min_segment: 64 * 1024, retry_delay: Duration::from_millis(50), stall: Duration::from_secs(5), ..Spec::new(url) }
}

#[test]
fn segmented_download_is_byte_exact() {
    let content = data(3 * 1024 * 1024 + 123);
    let server = serve(content.clone(), ServerOpts { ranges: true, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let ev = run_to_end(spec(&server.url), Shared::new(0), dir.path());
    assert_eq!(ev.first().unwrap(), &format!("probed test file.bin Some({}) true", content.len()));
    assert_eq!(ev.last().unwrap(), "done test file.bin");
    assert_eq!(fs::read(dir.path().join("test file.bin")).unwrap(), content);
    assert!(!dir.path().join("test file.bin.part").exists());
    // The probe plus at least the eight connections.
    assert!(server.requests.load(Ordering::SeqCst) >= 9);
}

#[test]
fn finished_connections_take_over_half_of_the_largest_segment() {
    // Resumed with one segment almost done: its connection finishes early
    // and takes over half of the other one.
    let content = data(800 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: true, rate: 400 * 1024, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("f.bin");
    let mut part = content.clone();
    part[390 * 1024..].fill(0);
    fs::write(part_path(&target), part).unwrap();
    let shared = Shared::new(0);
    let s = Spec {
        target: Some(target.clone()),
        size: Some(content.len() as u64),
        segments: vec![Segment { start: 0, end: 400 * 1024, pos: 390 * 1024 }, Segment::new(400 * 1024, 800 * 1024)],
        connections: 2,
        ..spec(&server.url)
    };
    let ev = run_to_end(s, shared.clone(), dir.path());
    assert_eq!(ev.last().unwrap(), "done f.bin");
    assert_eq!(fs::read(&target).unwrap(), content);
    assert!(server.peak.load(Ordering::SeqCst) >= 2, "connections ran side by side");
    let segs = shared.segments();
    assert!(segs.len() > 2, "the finished connection split the rest: {segs:?}");
    assert!(segs.iter().all(Segment::done));
    // Contiguous, covering the file.
    assert_eq!(segs.first().unwrap().start, 0);
    for w in segs.windows(2) {
        assert_eq!(w[0].end, w[1].start);
    }
    assert_eq!(segs.last().unwrap().end, content.len() as u64);
}

#[test]
fn pause_and_resume_where_it_stopped() {
    let content = data(1024 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: true, rate: 256 * 1024, etag: Some("\"v1\"".into()), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let shared = Shared::new(0);
    let sh = shared.clone();
    thread::spawn(move || {
        while sh.received() < 200 * 1024 {
            thread::sleep(Duration::from_millis(20));
        }
        sh.stop();
    });
    let s = Spec { connections: 2, ..spec(&server.url) };
    let ev = run_to_end(s, shared.clone(), dir.path());
    assert_eq!(ev.last().unwrap(), "paused");
    let segs = shared.segments();
    let got: u64 = segs.iter().map(Segment::received).sum();
    assert!(got >= 200 * 1024 && got < content.len() as u64, "{got}");
    assert!(dir.path().join("test file.bin.part").exists());

    // Resume, faster now.
    server.opts.lock().unwrap().rate = 0;
    let s = Spec {
        target: Some(dir.path().join("test file.bin")),
        size: Some(content.len() as u64),
        segments: segs,
        validator: http::Validator { etag: Some("\"v1\"".into()), last_modified: None },
        ..spec(&server.url)
    };
    let shared = Shared::new(0);
    let ev = run_to_end(s, shared.clone(), dir.path());
    assert_eq!(ev, ["running None", "done test file.bin"]);
    assert_eq!(fs::read(dir.path().join("test file.bin")).unwrap(), content);
}

#[test]
fn a_changed_file_restarts_from_the_beginning() {
    let content = data(300 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: true, etag: Some("\"v2\"".into()), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("f.bin");
    fs::write(part_path(&target), vec![0u8; content.len()]).unwrap();
    let s = Spec {
        target: Some(target.clone()),
        size: Some(content.len() as u64),
        segments: vec![Segment { start: 0, end: content.len() as u64, pos: 1000 }],
        validator: http::Validator { etag: Some("\"v1\"".into()), last_modified: None },
        ..spec(&server.url)
    };
    let ev = run_to_end(s, Shared::new(0), dir.path());
    assert_eq!(ev[0], "running Some(\"the file on the server changed\")");
    assert_eq!(fs::read(&target).unwrap(), content);
}

#[test]
fn servers_without_ranges_use_one_connection() {
    let content = data(500 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: false, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let ev = run_to_end(spec(&server.url), Shared::new(0), dir.path());
    assert_eq!(ev[0], format!("probed test file.bin Some({}) false", content.len()));
    assert_eq!(fs::read(dir.path().join("test file.bin")).unwrap(), content);
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}

#[test]
fn unknown_size_reads_to_the_end() {
    let content = data(200 * 1024 + 7);
    let server = serve(content.clone(), ServerOpts { ranges: false, chunked_unknown: true, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let ev = run_to_end(spec(&server.url), Shared::new(0), dir.path());
    assert_eq!(ev[0], "probed test file.bin None false");
    assert_eq!(fs::read(dir.path().join("test file.bin")).unwrap(), content);
}

#[test]
fn broken_connections_are_retried() {
    let content = data(2 * 1024 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: true, break_every: 3, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let s = Spec { retries: 20, ..spec(&server.url) };
    let ev = run_to_end(s, Shared::new(0), dir.path());
    assert_eq!(ev.last().unwrap(), "done test file.bin");
    assert_eq!(fs::read(dir.path().join("test file.bin")).unwrap(), content);
}

#[test]
fn missing_files_fail_at_once() {
    let server = serve(Vec::new(), ServerOpts { status: Some(404), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let ev = run_to_end(spec(&server.url), Shared::new(0), dir.path());
    assert_eq!(ev, ["failed the server doesn't have this file (404)"]);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[test]
fn headers_from_the_browser_go_with_every_request() {
    let content = data(1024 * 1024);
    let opts = ServerOpts { ranges: true, cookie: Some("session=abc".into()), ..Default::default() };
    let server = serve(content.clone(), opts);
    let dir = tempfile::tempdir().unwrap();
    let ev = run_to_end(spec(&server.url), Shared::new(0), dir.path());
    assert_eq!(ev.last().unwrap(), "failed the server refused access (403) — the link may have expired");

    let headers = vec![("Cookie".to_string(), "session=abc".to_string()), ("Referer".to_string(), "https://x.org/".to_string())];
    let ev = run_to_end(Spec { headers, ..spec(&server.url) }, Shared::new(0), dir.path());
    assert_eq!(ev.last().unwrap(), "done test file.bin");
    assert_eq!(fs::read(dir.path().join("test file.bin")).unwrap(), content);
}

#[test]
fn names_from_the_server_and_existing_files() {
    let server = serve(data(10), ServerOpts { ranges: true, disposition: Some("attachment; filename=\"report.pdf\"".into()), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("report.pdf"), b"old").unwrap();
    let ev = run_to_end(spec(&server.url), Shared::new(0), dir.path());
    // The reply asked for report.pdf; it exists, so the file gets a number.
    assert_eq!(ev.last().unwrap(), "done report (1).pdf");
    assert_eq!(fs::read(dir.path().join("report.pdf")).unwrap(), b"old");
}

#[test]
fn per_download_limit_holds() {
    let content = data(300 * 1024);
    let server = serve(content, ServerOpts { ranges: true, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let t = std::time::Instant::now();
    run_to_end(spec(&server.url), Shared::new(400 * 1024), dir.path());
    let secs = t.elapsed().as_secs_f64();
    assert!(secs > 0.5, "300 KB at 400 KB/s took {secs} s");
}

#[test]
fn splitting_and_initial_segments() {
    let segs = initial_segments(Some(1000), true, 4, 100);
    assert_eq!(segs.len(), 4);
    assert_eq!(segs[3], Segment::new(750, 1000));
    assert_eq!(initial_segments(Some(150), true, 4, 100).len(), 1);
    assert_eq!(initial_segments(None, false, 4, 100), [Segment::new(0, UNKNOWN_END)]);
    let mut slots = vec![Slot { seg: Segment::new(0, 1000), owner: 1, touched: Instant::now(), failures: 0, not_before: None }];
    assert_eq!(split_largest(&mut slots, 100), Some(1));
    assert_eq!(slots[0].seg.end, 500);
    assert_eq!(slots[1].seg, Segment::new(500, 1000));
    slots[0].seg.pos = 450;
    slots[1].owner = 2;
    slots[1].seg.pos = 900;
    assert_eq!(split_largest(&mut slots, 100), None, "nothing left worth splitting");
}

#[test]
fn unique_names() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.tar.gz");
    assert_eq!(unique_path(&p, |_| false), p);
    fs::write(&p, b"").unwrap();
    assert_eq!(unique_path(&p, |_| false), dir.path().join("a (1).tar.gz"));
    let taken = dir.path().join("a (1).tar.gz");
    assert_eq!(unique_path(&p, |q| q == taken), dir.path().join("a (2).tar.gz"));
}

/// `ODM_TEST_URL=https://… cargo test real_server -- --nocapture`: a real
/// server over HTTPS, with several connections if it serves ranges.
#[test]
fn real_server() {
    let Some(url) = std::env::var_os("ODM_TEST_URL") else { return };
    let dir = tempfile::tempdir().unwrap();
    let agent = http::agent(&http::NetConfig::default()).unwrap();
    let probe = http::probe(&agent, &url.to_string_lossy(), &Vec::new()).unwrap();
    println!("{probe:?}");
    let shared = Shared::new(0);
    let ev = run_to_end(Spec { min_segment: 256 * 1024, ..spec(&url.to_string_lossy()) }, shared.clone(), dir.path());
    println!("{ev:?} in {} parts", shared.segments().len());
    assert!(ev.last().unwrap().starts_with("done"));
    let file = dir.path().join(&probe.name);
    assert_eq!(fs::metadata(&file).unwrap().len(), probe.size.unwrap_or(0).max(shared.received()));
}
