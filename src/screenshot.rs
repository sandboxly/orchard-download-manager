//! `--screenshot OUT.png|OUT.webp`: a state drawn headlessly (docs, and
//! checking the look without a window): a demo list with downloads in
//! every state, and any dialog.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use oc_grid::{Frame, HeadlessRenderer};
use orchard_download_manager::App;
use orchard_download_manager::app::Modal;
use orchard_download_manager::checksum::Expected;
use orchard_download_manager::download::{Download, Run, Status};
use orchard_download_manager::engine::{Segment, Shared};
use orchard_download_manager::ui;

#[derive(Default)]
struct Options {
    size: (u16, u16),
    theme: Option<String>,
    modal: Option<String>,
    empty: bool,
    cursor: usize,
}

fn parse(args: &[String]) -> Options {
    let mut o = Options { size: (120, 40), cursor: 0, ..Options::default() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                if let Some((c, r)) = it.next().and_then(|s| s.split_once('x')) {
                    o.size = (c.parse().unwrap_or(o.size.0), r.parse().unwrap_or(o.size.1));
                }
            }
            "--theme" => o.theme = it.next().cloned(),
            "--dialog" => o.modal = it.next().cloned(),
            "--empty" => o.empty = true,
            "--cursor" => o.cursor = it.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            _ => {}
        }
    }
    o
}

pub fn run(args: &[String]) -> ExitCode {
    let Some(out) = args.first().map(PathBuf::from) else {
        eprintln!("--screenshot needs an output file");
        return ExitCode::FAILURE;
    };
    let o = parse(&args[1..]);
    let mut app = App::new(Arc::new(|| {}), false);
    if let Some(t) = &o.theme {
        let name = t.clone();
        app.update_settings(|s| s.color_scheme = name);
    }
    let mut r = match HeadlessRenderer::new(app.theme.font.clone(), 2.0) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("renderer: {e}");
            return ExitCode::FAILURE;
        }
    };
    let m = r.renderer().metrics();
    app.resized(o.size.0, o.size.1, (m.width as f32, m.height as f32));
    if !o.empty {
        demo(&mut app);
        let id = app.downloads.get(o.cursor).map(|d| d.id);
        app.cursor = id;
        app.scroll_to_cursor();
    }
    match o.modal.as_deref() {
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
            app.fonts = Some(oc_grid::FontLibrary::system().families(true));
        }
        Some("remove") => app.ask_remove(true),
        Some("properties") => ui::dialogs::open_properties(&mut app),
        _ => {}
    }
    let mut frame = Frame::new(o.size.0, o.size.1, app.theme.colors.background);
    ui::draw(&app, &mut frame);
    let renderer = r.renderer();
    orchard_download_manager::icons::upload_missing(&frame, &mut Default::default(), |id, w, h, rgba| {
        if let Err(e) = renderer.set_image(id, w, h, rgba) {
            eprintln!("icon: {e}");
        }
    });
    let saved = r.render(&frame).map_err(|e| e.to_string()).and_then(|img| save(&img, &out));
    match saved {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("screenshot: {e}");
            ExitCode::FAILURE
        }
    }
}

/// PNG, or lossless WebP for a `.webp` name.
fn save(img: &oc_grid::RgbaImage, out: &Path) -> Result<(), String> {
    let webp = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("webp"));
    if !webp {
        return img.save_png(out).map_err(|e| e.to_string());
    }
    let buf = image::RgbaImage::from_raw(img.width, img.height, img.data.clone()).ok_or("bad image size")?;
    buf.save_with_format(out, image::ImageFormat::WebP).map_err(|e| e.to_string())
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
