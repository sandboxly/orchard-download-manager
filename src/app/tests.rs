//! The app with downloads from a local server: the queue, pausing and
//! resuming, removing, checksums, the schedule, and the dialogs by keys.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::engine::tests::{ServerOpts, data, serve};
use crate::input::Mods;

fn app(dir: &Path) -> App {
    let mut app = App::new(Arc::new(|| {}), false);
    app.settings.download_dir = Some(dir.to_path_buf());
    app.settings.remember_last_folder = false;
    app.settings.prevent_sleep = false;
    app.settings.retry_delay_secs = 1;
    app.resized(120, 40, (8.0, 16.0));
    app
}

/// Run the app's loop until `cond` (or fail after a while).
fn settle(app: &mut App, what: &str, cond: impl Fn(&App) -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !cond(app) {
        assert!(Instant::now() < until, "timed out waiting for: {what}");
        app.process();
        app.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn status(app: &App) -> Vec<Status> {
    app.downloads.iter().map(|d| d.status).collect()
}

fn keys(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(&KeyEvent::char(c));
    }
}

fn key(app: &mut App, n: Named) {
    app.handle_key(&KeyEvent::named(n));
}

fn start() -> AddOptions {
    AddOptions { start: true, ..AddOptions::default() }
}

#[test]
fn a_link_downloads_into_the_download_folder() {
    let content = data(700 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: true, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    assert_eq!(app.add_links(std::slice::from_ref(&server.url), start()), 1);
    settle(&mut app, "done", |a| a.downloads[0].status == Status::Done);
    let d = &app.downloads[0];
    assert_eq!(d.name, "test file.bin");
    assert_eq!(d.size, Some(content.len() as u64));
    assert_eq!(d.progress(), Some(1.0));
    assert_eq!(std::fs::read(dir.path().join("test file.bin")).unwrap(), content);
    assert_eq!(app.toast.as_ref().map(|t| t.text.as_str()), Some("Downloaded test file.bin"));
}

#[test]
fn the_queue_runs_as_many_as_allowed() {
    let server = serve(data(200 * 1024), ServerOpts { ranges: true, rate: 400 * 1024, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.max_active = 1;
    let links: Vec<String> = (0..3).map(|i| format!("{}?n={i}", server.url)).collect();
    app.add_links(&links, start());
    assert_eq!(status(&app), [Status::Running, Status::Queued, Status::Queued]);
    settle(&mut app, "the first done", |a| a.downloads[0].status == Status::Done);
    assert_eq!(app.running(), 1, "the next one took the slot");
    settle(&mut app, "all done", |a| a.downloads.iter().all(|d| d.status == Status::Done));
    // Same name three times: numbered, nothing overwritten.
    for n in ["test file.bin", "test file (1).bin", "test file (2).bin"] {
        assert!(dir.path().join(n).exists(), "{n}");
    }
}

#[test]
fn pause_and_resume_finish_the_same_file() {
    let content = data(1024 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: true, rate: 300 * 1024, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.connections = 2;
    app.add_links(std::slice::from_ref(&server.url), start());
    settle(&mut app, "some bytes", |a| a.downloads[0].received_now() > 100 * 1024);
    let id = app.downloads[0].id;
    app.pause(&[id]);
    settle(&mut app, "paused", |a| a.downloads[0].status == Status::Paused);
    let got = app.downloads[0].received;
    assert!(got > 0 && got < content.len() as u64);
    assert!(!app.downloads[0].segments.is_empty());
    server.opts.lock().unwrap().rate = 0;
    app.start(&[id]);
    settle(&mut app, "done", |a| a.downloads[0].status == Status::Done);
    assert_eq!(std::fs::read(dir.path().join("test file.bin")).unwrap(), content);
}

#[test]
fn removing_a_running_download_deletes_its_partial_file() {
    let server = serve(data(2 * 1024 * 1024), ServerOpts { ranges: true, rate: 100 * 1024, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.add_links(std::slice::from_ref(&server.url), start());
    settle(&mut app, "running", |a| a.downloads[0].target.is_some());
    let part = engine::part_path(app.downloads[0].target.as_ref().unwrap());
    assert!(part.exists());
    let id = app.downloads[0].id;
    app.remove(&[id], false);
    settle(&mut app, "gone from the list", |a| a.downloads.is_empty());
    settle(&mut app, "part file deleted", |_| !part.exists());
}

#[test]
fn checksums_are_checked_after_downloading() {
    let content = b"abc".to_vec();
    let server = serve(content, ServerOpts { ranges: true, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let good = Expected::parse("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let bad = Expected::parse("900150983cd24fb0d6963f7d28e17f73");
    app.add_links(&[format!("{}?a", server.url)], AddOptions { checksum: good, ..start() });
    app.add_links(&[format!("{}?b", server.url)], AddOptions { checksum: bad, ..start() });
    settle(&mut app, "both verified", |a| a.downloads.iter().all(|d| d.verified.is_some()));
    assert_eq!(app.downloads[0].verified, Some(true));
    assert_eq!(app.downloads[1].verified, Some(false));
}

#[test]
fn a_missing_file_fails_with_a_reason() {
    let server = serve(Vec::new(), ServerOpts { status: Some(404), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.add_links(std::slice::from_ref(&server.url), start());
    settle(&mut app, "failed", |a| a.downloads[0].status == Status::Failed);
    assert_eq!(app.downloads[0].error.as_deref(), Some("the server doesn't have this file (404)"));
    app.set_tab(Tab::Failed);
    assert_eq!(app.view(), [0]);
}

#[test]
fn files_sort_into_category_folders() {
    let server = serve(data(10), ServerOpts { ranges: true, disposition: Some("attachment; filename=\"movie.mkv\"".into()), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.sort_by_category = true;
    app.add_links(std::slice::from_ref(&server.url), start());
    settle(&mut app, "done", |a| a.downloads[0].status == Status::Done);
    assert!(dir.path().join("Video/movie.mkv").exists());
}

#[test]
fn the_schedule_window() {
    let mut s = Settings { schedule_start: "22:00".into(), schedule_stop: "06:00".into(), ..Settings::default() };
    assert!(in_window(&s, 0, 23 * 60));
    assert!(in_window(&s, 1, 3 * 60), "past midnight belongs to the evening before");
    assert!(!in_window(&s, 1, 12 * 60));
    s.schedule_days = [false, true, true, true, true, true, true];
    assert!(!in_window(&s, 0, 23 * 60), "not on Mondays");
    assert!(!in_window(&s, 1, 3 * 60), "Monday night either");
    s.schedule_stop = String::new();
    assert!(in_window(&s, 2, 23 * 60 + 59));
    assert!(!in_window(&s, 2, 21 * 60));
}

#[test]
fn queued_downloads_wait_for_the_schedule() {
    let server = serve(data(10), ServerOpts { ranges: true, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.schedule_enabled = true;
    app.settings.schedule_days = [false; 7];
    app.add_links(std::slice::from_ref(&server.url), start());
    assert_eq!(status(&app), [Status::Queued]);
    // By hand it starts anyway.
    let id = app.downloads[0].id;
    app.start(&[id]);
    settle(&mut app, "done", |a| a.downloads[0].status == Status::Done);
}

#[test]
fn adding_by_keys() {
    let server = serve(data(10), ServerOpts { ranges: true, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.fkey(2);
    assert!(matches!(app.modal, Some(Modal::Add(_))));
    keys(&mut app, &server.url);
    // Enter in the links field adds.
    key(&mut app, Named::Enter);
    assert!(app.modal.is_none());
    assert_eq!(app.downloads.len(), 1);
    settle(&mut app, "done", |a| a.downloads[0].status == Status::Done);
}

#[test]
fn the_add_dialog_explains_what_is_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    ui::add::open(&mut app, "");
    keys(&mut app, "not a link");
    key(&mut app, Named::Enter);
    let Some(Modal::Add(a)) = &app.modal else { panic!("closed") };
    assert!(a.error.as_deref().unwrap().starts_with("Add a link first"));
    assert!(app.downloads.is_empty());
    key(&mut app, Named::Escape);
    assert!(app.modal.is_none());
}

#[test]
fn batch_patterns_add_many() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.start_immediately = false;
    ui::add::open(&mut app, "https://x.org/img[01-12].jpg");
    key(&mut app, Named::Enter);
    assert_eq!(app.downloads.len(), 12);
    assert_eq!(app.downloads[11].name, "img12.jpg");
    assert!(app.downloads.iter().all(|d| d.status == Status::Paused));
}

#[test]
fn copied_links_open_the_add_dialog_for_files_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.clipboard_changed("see https://x.org/page.html");
    assert!(app.modal.is_none());
    app.clipboard_changed("get https://x.org/tool.zip now");
    let Some(Modal::Add(a)) = &app.modal else { panic!("no dialog") };
    assert_eq!(a.links.text, "https://x.org/tool.zip");
    assert!(app.attention);
}

#[test]
fn tabs_search_and_marks() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.start_immediately = false;
    app.add_links(&["https://x.org/a.zip".into(), "https://x.org/b.iso".into(), "https://y.org/c.zip".into()], AddOptions::default());
    app.downloads[1].status = Status::Done;
    key(&mut app, Named::Tab);
    assert_eq!(app.tab, Tab::Active);
    assert!(app.view().is_empty(), "all paused or done");
    app.handle_key(&KeyEvent::char('4'));
    assert_eq!(app.view(), [1]);
    app.handle_key(&KeyEvent::char('1'));
    keys(&mut app, "/x.org");
    assert_eq!(app.view(), [0, 1]);
    key(&mut app, Named::Escape);
    assert_eq!(app.view().len(), 3);
    // Filter by type: archives (zip, iso).
    app.cycle_category();
    while app.category != Some(Category::Archives) {
        app.cycle_category();
    }
    assert_eq!(app.view(), [0, 1, 2]);
    app.handle_key(&KeyEvent { key: Key::Char('a'), mods: primary(), text: None });
    assert_eq!(app.selection().len(), 3);
}

fn primary() -> Mods {
    if cfg!(target_os = "macos") { Mods { logo: true, ..Mods::default() } } else { Mods { ctrl: true, ..Mods::default() } }
}

#[test]
fn moving_in_the_queue() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.start_immediately = false;
    app.add_links(&["https://x.org/a".into(), "https://x.org/b".into()], AddOptions::default());
    assert_eq!(app.selected().unwrap().name, "b");
    app.handle_key(&KeyEvent::named(Named::Up).with(Mods { alt: true, ..Mods::default() }));
    let names: Vec<_> = app.downloads.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["b", "a"]);
}

#[test]
fn settings_change_by_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.fkey(9);
    // Down to Connections, into its controls: the first is "downloads at once".
    for _ in 0..3 {
        key(&mut app, Named::Down);
    }
    key(&mut app, Named::Tab);
    key(&mut app, Named::Right);
    assert_eq!(app.settings.max_active, 4);
    // Next: connections per download.
    key(&mut app, Named::Tab);
    key(&mut app, Named::Left);
    assert_eq!(app.settings.connections, 7);
    // Speed page: the mode list steps with ←/→ and the limiter follows.
    key(&mut app, Named::Escape);
    ui::settings::open(&mut app, Some(4));
    key(&mut app, Named::Tab);
    key(&mut app, Named::Left);
    assert_eq!(app.settings.traffic, settings::Traffic::Medium);
    assert_eq!(app.global.rate(), 2048 * 1024);
}

#[test]
fn a_bad_time_stays_in_its_field_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.schedule_enabled = true;
    ui::settings::open(&mut app, Some(7));
    key(&mut app, Named::Tab); // "Download on a schedule"
    key(&mut app, Named::Tab); // "Start at"
    app.handle_key(&KeyEvent::named(Named::Backspace).with(primary()));
    keys(&mut app, "25:00");
    key(&mut app, Named::Tab);
    let Some(Modal::Settings(s)) = &app.modal else { panic!("closed") };
    assert!(s.problem.as_deref().unwrap().contains("hours and minutes"));
    assert_eq!(app.settings.schedule_start, "01:00");
    // Fixed, it applies.
    app.handle_key(&KeyEvent::named(Named::Backspace).with(primary()));
    keys(&mut app, "2:5");
    key(&mut app, Named::Enter);
    assert_eq!(app.settings.schedule_start, "02:05");
    key(&mut app, Named::Escape);
    assert!(app.modal.is_none());
}

#[test]
fn every_dialog_draws_at_small_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.add_links(&["https://x.org/a.zip".into()], AddOptions::default());
    for (cols, rows) in [(120, 40), (60, 20), (30, 8)] {
        app.resized(cols, rows, (8.0, 16.0));
        for open in 0..6 {
            match open {
                0 => ui::add::open(&mut app, "https://x.org/b.zip https://x.org/c.zip"),
                1 => ui::settings::open(&mut app, None),
                2 => app.ask_remove(true),
                3 => ui::dialogs::open_properties(&mut app),
                4 => app.modal = Some(Modal::Help),
                _ => app.modal = None,
            }
            let mut f = oc_grid::Frame::new(cols, rows, app.theme.colors.background);
            ui::draw(&app, &mut f);
        }
        for page in 0..ui::settings::PAGES.len() {
            ui::settings::open(&mut app, Some(page));
            let mut f = oc_grid::Frame::new(cols, rows, app.theme.colors.background);
            ui::draw(&app, &mut f);
        }
    }
}

#[test]
fn choosing_a_folder_from_the_add_dialog() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Sub")).unwrap();
    let mut app = app(dir.path());
    ui::add::open(&mut app, "https://x.org/a.zip");
    // Links → Save to → Choose…
    let Some(Modal::Add(a)) = &mut app.modal else { panic!() };
    a.focus = Some(ui::add::Id::Choose);
    key(&mut app, Named::Enter);
    let listed = |app: &App| matches!(&app.modal, Some(Modal::Add(a)) if a.picker.as_ref().is_some_and(|p| p.listed()));
    settle(&mut app, "listed", listed);
    // Row 0 is "..", then "Sub".
    key(&mut app, Named::End);
    key(&mut app, Named::Enter);
    settle(&mut app, "in Sub", |app| {
        matches!(&app.modal, Some(Modal::Add(a)) if a.picker.as_ref().is_some_and(|p| p.place == ui::picker::Place::Dir(dir.path().join("Sub"))))
    });
    key(&mut app, Named::F(2));
    let Some(Modal::Add(a)) = &app.modal else { panic!() };
    assert!(a.picker.is_none());
    assert_eq!(ui::add::expand_tilde(&a.dir.text), dir.path().join("Sub"));
    key(&mut app, Named::Enter);
    assert_eq!(app.downloads[0].dir.as_deref(), Some(dir.path().join("Sub").as_path()));
}

#[test]
fn downloads_from_the_browser_keep_their_cookies() {
    let content = data(300 * 1024);
    let server = serve(content.clone(), ServerOpts { ranges: true, cookie: Some("session=abc".into()), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let link = crate::ipc::Link { url: server.url.clone(), headers: vec![("Cookie".into(), "session=abc".into())], name: Some("report.bin".into()) };
    let h = crate::ipc::Handoff { items: vec![link], browser: true, ..Default::default() };

    // The Add dialog first, with the browser's name; adding keeps the headers.
    app.handoff(h.clone());
    let Some(Modal::Add(a)) = &app.modal else { panic!("the Add dialog opens") };
    assert_eq!(a.name.text, "report.bin");
    key(&mut app, Named::Enter);
    assert!(app.modal.is_none());
    settle(&mut app, "done", |a| a.downloads[0].status == Status::Done);
    assert_eq!(std::fs::read(dir.path().join("report.bin")).unwrap(), content);

    // Straight into the list when the settings say so.
    app.settings.browser_ask = false;
    app.settings.start_immediately = false;
    app.handoff(h);
    assert!(app.modal.is_none());
    assert_eq!(app.downloads.len(), 2);
    assert_eq!(app.downloads[1].status, Status::Paused);
    assert_eq!(app.downloads[1].headers, [("Cookie".to_string(), "session=abc".to_string())]);
}

#[test]
fn links_from_another_start_open_the_add_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.settings.browser_ask = false;
    app.external_links(vec!["see https://x.org/a.zip".into()]);
    assert!(matches!(&app.modal, Some(Modal::Add(a)) if a.links.text == "https://x.org/a.zip"), "only the browser's links skip the dialog");
    assert!(app.raise);
}
