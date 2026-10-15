//! `--screenshot OUT.png|OUT.webp`: a state drawn headlessly (docs, and
//! checking the look without a window): a demo list with downloads in
//! every state, and any dialog, with its folder picker open.

use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orchard_grid::Frame;
use orchard_kit::input::{KeyEvent, Mods};
use orchard_download_manager::App;
use orchard_download_manager::app::Modal;
use orchard_download_manager::checksum::Expected;
use orchard_download_manager::download::{Download, Run, Status};
use orchard_download_manager::engine::{Segment, Shared};
use orchard_download_manager::ui;
use orchard_shell::shot;

/// The options only this app's screenshots have (the common ones are
/// `orchard_shell::shot`'s).
#[derive(Default)]
struct Options {
    empty: bool,
    cursor: usize,
    /// The dialog's folder picker ("Choose…").
    picker: bool,
    /// The picker's bookmark menu (⌘D / Ctrl+D).
    picker_menu: bool,
}

pub fn run(args: &[String]) -> ExitCode {
    let mut o = Options::default();
    let shot = match shot::parse(args, (120, 40), |a, it| match a {
        "--empty" => o.empty = true,
        "--cursor" => o.cursor = it.next().and_then(|s| s.parse().ok()).unwrap_or(0),
        "--picker" => o.picker = true,
        "--picker-menu" => (o.picker, o.picker_menu) = (true, true),
        _ => {}
    }) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let mut app = App::new(
        Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        }),
        false,
    );
    if let Some(t) = &shot.theme {
        let name = t.clone();
        app.update_settings(|s| s.color_scheme = name);
    }
    let mut r = match shot::renderer(app.theme.font.clone()) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let (cols, rows) = shot.size;
    app.resized(cols, rows, shot::cell_px(&mut r));
    if !o.empty {
        demo(&mut app);
        let id = app.downloads.get(o.cursor).map(|d| d.id);
        app.cursor = id;
        app.scroll_to_cursor();
    }
    match shot.dialog.as_deref() {
        Some("help") => app.modal = Some(Modal::Help),
        Some("add") => {
            ui::add::open(&mut app, "https://downloads.example.org/releases/orchard-commander-2.0.1-macOS.dmg");
            if let Some(Modal::Add(a)) = &mut app.modal {
                a.probe_due = None;
                a.lookup = ui::add::Lookup::Found(
                    a.links.text.clone(),
                    orchard_download_manager::engine::http::Probe {
                        name: "orchard-commander-2.0.1-macOS.dmg".into(),
                        size: Some(48_234_496),
                        resumable: true,
                        ..Default::default()
                    },
                );
            }
        }
        Some("batch") => ui::add::open(&mut app, "https://media.example.org/lectures/lecture-[01-12].mp4"),
        Some(s) if s.starts_with("settings") => {
            let page = s.split_once(':').and_then(|(_, p)| {
                let p = p.to_lowercase();
                ui::settings::PAGES.iter().position(|x| x.to_lowercase().starts_with(&p))
            });
            ui::settings::open(&mut app, page);
            app.fonts = Some(orchard_grid::FontLibrary::system().families(true));
        }
        Some("remove") => app.ask_remove(true),
        Some("properties") => ui::dialogs::open_properties(&mut app),
        _ => {}
    }
    if o.picker {
        open_picker(&mut app, o.picker_menu, &rx);
    }
    let mut frame = Frame::new(cols, rows, app.theme.colors.background);
    ui::draw(&app, &mut frame);
    let renderer = r.renderer();
    orchard_download_manager::icons::upload_missing(&frame, &mut Default::default(), |id, w, h, rgba| {
        if let Err(e) = renderer.set_image(id, w, h, rgba) {
            eprintln!("icon: {e}");
        }
    });
    shot::finish(&mut r, &frame, &shot.out)
}

/// Press the dialog's "Choose…" and wait for the picker's listing, places
/// and bookmarks (a few seconds at most).
fn open_picker(app: &mut App, menu: bool, rx: &mpsc::Receiver<()>) {
    let mut modal = app.modal.take();
    match &mut modal {
        Some(Modal::Add(a)) => ui::add::choose(app, a),
        Some(Modal::Properties(p)) => p.choose(app),
        Some(Modal::Settings(d)) => ui::settings::choose_folder(app, d, ui::settings::Folder::Downloads),
        _ => {}
    }
    app.modal = modal;
    let until = Instant::now() + Duration::from_secs(5);
    while app.modal.as_mut().and_then(Modal::picker).is_some_and(|p| p.loading()) && Instant::now() < until {
        let _ = rx.recv_timeout(until.saturating_duration_since(Instant::now()));
        app.process();
    }
    if menu {
        let d = KeyEvent::char('d').with(Mods { ctrl: true, logo: true, ..Default::default() });
        app.handle_key(&d);
    }
}

const MB: u64 = 1024 * 1024;

/// (name, size, status, fraction done, speed in MB/s, connections)
const DEMO: &[(&str, u64, Status, f64, f64, usize)] = &[
    ("lecture-07-distributed-systems.mp4", 1_843 * MB, Status::Running, 0.62, 11.8, 8),
    ("kernel-source-6.17.tar.xz", 148 * MB, Status::Running, 0.27, 4.3, 6),
    ("orchard-commander-2.0.1-macOS.dmg", 46 * MB, Status::Done, 1.0, 0.0, 0),
    ("photos-2026-summer.zip", 3_420 * MB, Status::Paused, 0.41, 0.0, 0),
    ("dataset-2026-q3.csv.gz", 812 * MB, Status::Queued, 0.0, 0.0, 0),
    ("podcast-episode-142.mp3", 74 * MB, Status::Done, 1.0, 0.0, 0),
    ("annual-report-2025.pdf", 12 * MB, Status::Done, 1.0, 0.0, 0),
    ("conference-keynote-4k.mkv", 6_210 * MB, Status::Queued, 0.0, 0.0, 0),
    ("font-collection.zip", 0, Status::Failed, 0.0, 0.0, 0),
    ("orchard-music-player-1.0.1.dmg", 31 * MB, Status::Done, 1.0, 0.0, 0),
    ("vm-image-server.qcow2", 9_870 * MB, Status::Paused, 0.08, 0.0, 0),
];

fn demo(app: &mut App) {
    let now = orchard_download_manager::format::now_unix();
    let base = dirs::home_dir().unwrap_or_default().join("Downloads");
    for (i, &(name, size, status, frac, speed, conns)) in DEMO.iter().enumerate() {
        let url = format!("https://downloads.example.org/files/{name}");
        let mut d = Download::new(i as u64 + 1, url);
        d.status = status;
        d.added = now - 3600 * (DEMO.len() - i) as i64;
        d.target = Some(base.join(name));
        d.size = (size > 0).then_some(size);
        d.resumable = Some(true);
        d.received = (size as f64 * frac) as u64;
        match status {
            Status::Done => {
                d.finished = Some(d.added + 900);
                if i == 2 {
                    d.checksum = Expected::parse("sha256:9f2a6c1e0b7d4c3a8e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5");
                    d.verified = Some(true);
                }
                if i == 5 {
                    d.checksum = Expected::parse("900150983cd24fb0d6963f7d28e17f72");
                    d.verified = Some(false);
                }
            }
            Status::Failed => {
                d.target = None;
                d.resumable = None;
                d.error = Some("the server refused access (403) — the link may have expired".into());
            }
            Status::Paused => d.segments = parts(size, frac, 8),
            Status::Running => {
                let segs = parts(size, frac, conns);
                let shared = Shared::new(0);
                shared.set_segments(&segs, conns);
                let mut run = Run::new(shared);
                run.speed = speed * MB as f64;
                d.run = Some(run);
                // A plausible speed graph, full width.
                d.history = (0..orchard_download_manager::download::HISTORY)
                    .map(|k| {
                        let t = k as f32;
                        let ramp = (t / 12.0).min(1.0);
                        (speed as f32 * MB as f32) * ramp * (0.82 + 0.12 * (t * 0.31).sin() + 0.06 * (t * 1.7).cos())
                    })
                    .collect();
            }
            Status::Queued => {}
        }
        app.downloads.push(d);
    }
    app.history = app.downloads[0].history.iter().zip(&app.downloads[1].history).map(|(a, b)| a + b).collect();
}

/// `n` segments of a file `frac` done, the first ones a little further.
fn parts(size: u64, frac: f64, n: usize) -> Vec<Segment> {
    let n = n.max(1) as u64;
    let step = size / n;
    (0..n)
        .map(|i| {
            let start = i * step;
            let end = if i + 1 == n { size } else { start + step };
            let skew = 1.15 - 0.3 * (i as f64 / n as f64);
            let pos = start + ((end - start) as f64 * (frac * skew).min(1.0)) as u64;
            Segment { start, end, pos }
        })
        .collect()
}
