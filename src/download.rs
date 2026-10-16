//! A download in the list, and the list's file (`downloads.json`).

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::category::Category;
use crate::checksum::Expected;
use crate::engine::http::{Headers, Validator};
use crate::engine::{Segment, Shared};
use crate::settings::Conflict;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Waits for a free slot (or the schedule).
    Queued,
    Running,
    Paused,
    Done,
    Failed,
}

/// Speed samples kept for the graph (one per tick).
pub const HISTORY: usize = 240;

/// A running download's live state (not saved).
pub struct Run {
    pub shared: Arc<Shared>,
    pub started: Instant,
    /// Bytes per second, smoothed.
    pub speed: f64,
    last: (Instant, u64),
    /// Asked to stop, waiting for `Paused`.
    pub stopping: bool,
    /// Removed while stopping: forget it (and delete its files) once stopped.
    pub then: Then,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    Pause,
    /// Back to the queue (the schedule closed).
    Queue,
    /// Start again (settings or address changed).
    Restart,
    Remove { delete_files: bool },
}

impl Run {
    pub fn new(shared: Arc<Shared>) -> Run {
        let now = Instant::now();
        let got = shared.received();
        Run { shared, started: now, speed: 0.0, last: (now, got), stopping: false, then: Then::Pause }
    }

    /// Update the speed from the bytes received since the last sample.
    pub fn sample(&mut self, now: Instant) -> f64 {
        let got = self.shared.received();
        let dt = now.duration_since(self.last.0).as_secs_f64();
        if dt < 0.05 {
            return self.speed;
        }
        let rate = got.saturating_sub(self.last.1) as f64 / dt;
        // Quick to rise from nothing, smooth after.
        let k = if self.speed == 0.0 { 1.0 } else { (dt / 2.0).min(1.0) };
        self.speed += (rate - self.speed) * k;
        self.last = (now, got);
        self.speed
    }
}

#[derive(Serialize, Deserialize)]
pub struct Download {
    pub id: u64,
    pub url: String,
    /// The file name (from the server, the URL, or chosen).
    pub name: String,
    /// The name was chosen by hand: the server's doesn't replace it.
    #[serde(default)]
    pub name_fixed: bool,
    /// The folder; `None`: by the file's category when the name is known.
    pub dir: Option<PathBuf>,
    /// Where the file goes, decided when it started.
    pub target: Option<PathBuf>,
    pub size: Option<u64>,
    pub received: u64,
    #[serde(default)]
    pub segments: Vec<Segment>,
    pub resumable: Option<bool>,
    #[serde(default)]
    pub validator: Validator,
    pub status: Status,
    pub error: Option<String>,
    pub added: i64,
    pub finished: Option<i64>,
    #[serde(default)]
    pub headers: Headers,
    /// Overrides of the settings.
    pub connections: Option<u32>,
    /// Bytes per second.
    pub speed_limit: Option<u64>,
    pub checksum: Option<Expected>,
    /// Whether the finished file matched `checksum`.
    pub verified: Option<bool>,
    /// What to do if the file already exists, when decided for this one
    /// (in the Add dialog or when asked); else the settings say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Conflict>,
    #[serde(skip)]
    pub run: Option<Run>,
    #[serde(skip)]
    pub history: VecDeque<f32>,
}

impl Download {
    pub fn new(id: u64, url: String) -> Download {
        let name = crate::engine::http::sanitize_filename(&crate::engine::http::url_filename(&url));
        Download {
            id,
            url,
            name,
            name_fixed: false,
            dir: None,
            target: None,
            size: None,
            received: 0,
            segments: Vec::new(),
            resumable: None,
            validator: Validator::default(),
            status: Status::Queued,
            error: None,
            added: crate::format::now_unix(),
            finished: None,
            headers: Vec::new(),
            connections: None,
            speed_limit: None,
            checksum: None,
            verified: None,
            conflict: None,
            run: None,
            history: VecDeque::new(),
        }
    }

    pub fn category(&self) -> Category {
        Category::of(&self.name)
    }

    /// Bytes on disk now.
    pub fn received_now(&self) -> u64 {
        self.run.as_ref().map_or(self.received, |r| r.shared.received())
    }

    pub fn speed(&self) -> f64 {
        self.run.as_ref().map_or(0.0, |r| r.speed)
    }

    /// 0…1, if the size is known.
    pub fn progress(&self) -> Option<f64> {
        if self.status == Status::Done {
            return Some(1.0);
        }
        let size = self.size.filter(|&s| s > 0)?;
        Some((self.received_now() as f64 / size as f64).min(1.0))
    }

    /// Seconds left at the current speed.
    pub fn eta(&self) -> Option<f64> {
        let size = self.size?;
        let speed = self.speed();
        (self.status == Status::Running && speed > 1.0).then(|| size.saturating_sub(self.received_now()) as f64 / speed)
    }

    /// The segments now (live while running).
    pub fn segments_now(&self) -> Vec<Segment> {
        match &self.run {
            Some(r) => {
                let s = r.shared.segments();
                if s.is_empty() { self.segments.clone() } else { s }
            }
            None => self.segments.clone(),
        }
    }

    /// Copy the live state into the saved fields.
    pub fn sync_from_run(&mut self) {
        if let Some(r) = &self.run {
            let segs = r.shared.segments();
            if !segs.is_empty() {
                self.segments = segs;
            }
            self.received = r.shared.received();
        }
    }

    /// The file this download makes, finished or not.
    pub fn file(&self) -> Option<&Path> {
        self.target.as_deref()
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, Status::Running | Status::Queued)
    }

    pub fn push_history(&mut self) {
        let v = self.speed() as f32;
        if self.history.len() >= HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(v);
    }
}

#[derive(Serialize, Deserialize, Default)]
struct ListFile {
    next_id: u64,
    downloads: Vec<Download>,
}

pub fn list_file(dir: &Path) -> PathBuf {
    dir.join("downloads.json")
}

/// The saved list; running downloads come back paused-to-resume (queued).
pub fn load(dir: &Path) -> (Vec<Download>, u64) {
    let Ok(text) = fs::read_to_string(list_file(dir)) else { return (Vec::new(), 1) };
    let Ok(mut f) = serde_json::from_str::<ListFile>(&text) else {
        // Keep the unreadable file for a look, start empty.
        let _ = fs::rename(list_file(dir), dir.join("downloads.json.broken"));
        return (Vec::new(), 1);
    };
    for d in &mut f.downloads {
        if d.status == Status::Running {
            d.status = Status::Queued;
        }
    }
    let next = f.downloads.iter().map(|d| d.id + 1).max().unwrap_or(1).max(f.next_id);
    (f.downloads, next)
}

pub fn save(dir: &Path, downloads: &[Download], next_id: u64) -> std::io::Result<()> {
    #[derive(Serialize)]
    struct Out<'a> {
        next_id: u64,
        downloads: &'a [Download],
    }
    fs::create_dir_all(dir)?;
    // Only for this user: downloads from the browser keep its cookies.
    crate::ipc::write_private(&list_file(dir), &serde_json::to_string_pretty(&Out { next_id, downloads }).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_round_trip_requeues_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Download::new(4, "https://x.org/a%20b.zip".into());
        assert_eq!(a.name, "a b.zip");
        a.status = Status::Running;
        a.segments = vec![Segment { start: 0, end: 10, pos: 5 }];
        let mut b = Download::new(7, "https://x.org/c.iso".into());
        b.status = Status::Done;
        save(dir.path(), &[a, b], 8).unwrap();
        let (list, next) = load(dir.path());
        assert_eq!(next, 8);
        assert_eq!(list[0].status, Status::Queued);
        assert_eq!(list[0].segments[0].pos, 5);
        assert_eq!(list[1].status, Status::Done);
    }

    #[test]
    fn a_broken_file_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(list_file(dir.path()), "{nope").unwrap();
        let (list, next) = load(dir.path());
        assert!(list.is_empty() && next == 1);
        assert!(dir.path().join("downloads.json.broken").exists());
    }
}
