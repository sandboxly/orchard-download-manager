//! All of the app's state and behavior. The binary owns only the window
//! and the GPU; it feeds events in and draws `ui::draw(&app, …)`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use orchard_kit::input::{Key, KeyEvent, MouseEvent, MouseKind, Named};
use orchard_kit::tasks::{Tasks, Waker};
use orchard_kit::widgets::TextField;
use orchard_theme::{Theme, schemes};

use crate::category::Category;
use crate::checksum::Expected;
use crate::download::{self, Download, Run, Status, Then};
use crate::engine::http::{self, NetConfig};
use crate::engine::limiter::Limiter;
use crate::engine::{self, Event, Reporter, Shared, Spec};
use crate::format;
use crate::settings::{self, ProxyMode, Settings, WhenDone};
use crate::system;
use crate::ui::{self, add::AddDialog, dialogs, settings::SettingsDialog};

/// The list's status tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    All,
    /// Running or waiting.
    Active,
    Paused,
    Done,
    Failed,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::All, Tab::Active, Tab::Paused, Tab::Done, Tab::Failed];

    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Active => "Active",
            Tab::Paused => "Paused",
            Tab::Done => "Completed",
            Tab::Failed => "Failed",
        }
    }

    pub fn shows(self, s: Status) -> bool {
        match self {
            Tab::All => true,
            Tab::Active => matches!(s, Status::Running | Status::Queued),
            Tab::Paused => s == Status::Paused,
            Tab::Done => s == Status::Done,
            Tab::Failed => s == Status::Failed,
        }
    }
}

/// The quick filter (/): downloads whose name or address has every word.
#[derive(Clone, Debug, Default)]
pub struct Search {
    pub field: TextField,
    pub editing: bool,
}

pub enum Modal {
    Add(Box<AddDialog>),
    Settings(SettingsDialog),
    Remove(dialogs::Remove),
    Properties(dialogs::Properties),
    Checksum(dialogs::Checksum),
    Help,
}

impl Modal {
    /// The folder picker open over the dialog.
    pub fn picker(&mut self) -> Option<&mut orchard_kit::picker::Picker> {
        match self {
            Modal::Add(a) => a.picker.as_mut(),
            Modal::Properties(p) => p.picker.as_mut(),
            Modal::Settings(s) => s.picker.as_mut().map(|(_, p)| p),
            Modal::Remove(_) | Modal::Checksum(_) | Modal::Help => None,
        }
    }
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

/// What the app does when the last download finished, after a countdown.
pub struct Pending {
    pub action: WhenDone,
    pub at: Instant,
}

/// How long the "when done" countdown runs.
pub const WHEN_DONE_DELAY: Duration = Duration::from_secs(30);

/// How to add links.
#[derive(Clone, Debug, Default)]
pub struct AddOptions {
    /// `None`: the category's folder.
    pub dir: Option<PathBuf>,
    /// Only for a single link.
    pub name: Option<String>,
    pub checksum: Option<Expected>,
    pub start: bool,
    /// Request headers by link (from the browser).
    pub headers: HashMap<String, http::Headers>,
}

pub struct App {
    pub settings: Settings,
    config_dir: PathBuf,
    /// False in screenshot mode and tests: nothing is written.
    persist: bool,
    pub theme: Theme,
    /// The system is in dark mode (for following its appearance).
    pub system_dark: bool,
    pub downloads: Vec<Download>,
    next_id: u64,
    /// The selected download.
    pub cursor: Option<u64>,
    pub list_top: usize,
    pub marked: HashSet<u64>,
    pub tab: Tab,
    pub category: Option<Category>,
    pub search: Option<Search>,
    pub modal: Option<Modal>,
    pub toast: Option<Toast>,
    pub pending: Option<Pending>,
    pub tasks: Tasks<App>,
    pub global: Arc<Limiter>,
    agent: Result<ureq::Agent, String>,
    net: NetConfig,
    /// Total speed, one sample a second.
    pub history: VecDeque<f32>,
    pub cols: u16,
    pub rows: u16,
    pub cell_px: (f32, f32),
    pub quit: bool,
    /// The window should ask for attention / come to the front.
    pub attention: bool,
    pub raise: bool,
    dirty: bool,
    last_save: Instant,
    last_sample: Instant,
    /// The schedule's state at the last check (to act on changes only).
    schedule_open: Option<bool>,
    awake: Option<system::Awake>,
    clipboard_on: Arc<AtomicBool>,
    /// Monospaced font families for the settings (looked up on a worker).
    pub fonts: Option<Vec<String>>,
}

impl App {
    /// `persist`: read and write settings and the list, watch the clipboard.
    pub fn new(wake: Waker, persist: bool) -> App {
        let config_dir = settings::config_dir();
        let settings = if persist { Settings::load(&config_dir) } else { Settings::default() };
        let (downloads, next_id) = if persist { download::load(&config_dir) } else { (Vec::new(), 1) };
        let theme = Theme::new(&settings.appearance(), schemes::get_scheme(&orchard_theme::dirs::schemes_dir(), &settings.color_scheme));
        let net = net_config(&settings);
        let mut app = App {
            global: Arc::new(Limiter::new(settings.speed_limit())),
            agent: http::agent(&net),
            net,
            theme,
            system_dark: true,
            downloads,
            next_id,
            cursor: None,
            list_top: 0,
            marked: HashSet::new(),
            tab: Tab::All,
            category: None,
            search: None,
            modal: None,
            toast: None,
            pending: None,
            tasks: Tasks::new(3, wake),
            history: VecDeque::new(),
            cols: 100,
            rows: 36,
            cell_px: (8.0, 16.0),
            quit: false,
            attention: false,
            raise: false,
            dirty: false,
            last_save: Instant::now(),
            last_sample: Instant::now(),
            schedule_open: None,
            awake: None,
            clipboard_on: Arc::new(AtomicBool::new(false)),
            fonts: None,
            settings,
            config_dir,
            persist,
        };
        app.cursor = app.downloads.first().map(|d| d.id);
        app.apply_theme();
        if persist {
            app.clipboard_on.store(app.settings.clipboard_monitor, Ordering::Relaxed);
            let tasks = app.tasks.clone();
            crate::clipboard::watch(app.clipboard_on.clone(), move |text| tasks.post(move |app| app.clipboard_changed(&text)));
            app.forget_old();
            app.schedule();
        }
        app
    }

    /// The HTTP client, or why there is none (a bad proxy address).
    pub fn agent(&self) -> Result<ureq::Agent, String> {
        self.agent.clone()
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    // -- persistence ------------------------------------------------------

    fn changed(&mut self) {
        self.dirty = true;
    }

    /// Write the list (and settings) if anything changed.
    pub fn save(&mut self) {
        self.last_save = Instant::now();
        if !self.persist || !self.dirty {
            return;
        }
        self.dirty = false;
        for d in &mut self.downloads {
            d.sync_from_run();
        }
        if let Err(e) = download::save(&self.config_dir, &self.downloads, self.next_id) {
            log::warn!("saving the list: {e}");
        }
    }

    pub fn save_settings(&mut self) {
        if self.persist
            && let Err(e) = self.settings.save(&self.config_dir)
        {
            self.toast(format!("Can't save the settings: {}", format::io_reason(&e)), true);
        }
    }

    /// On quit: pause what runs (keeping it queued for next time) and save.
    pub fn shutdown(&mut self) {
        for d in &mut self.downloads {
            if let Some(r) = &d.run {
                r.shared.stop();
            }
        }
        // Give the connections a moment to write their last bytes.
        let until = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < until && self.downloads.iter().any(|d| d.run.as_ref().is_some_and(|r| r.shared.connections.load(Ordering::Relaxed) > 0)) {
            std::thread::sleep(Duration::from_millis(30));
        }
        for d in &mut self.downloads {
            d.sync_from_run();
            if d.run.take().is_some() {
                d.status = Status::Queued;
            }
        }
        self.dirty = true;
        self.save();
        self.save_settings();
        self.awake = None;
        if self.persist {
            crate::ipc::forget(&self.config_dir);
        }
    }

    /// Finished downloads older than the setting leave the list.
    fn forget_old(&mut self) {
        let days = self.settings.forget_after_days;
        if days == 0 {
            return;
        }
        let cutoff = format::now_unix() - days as i64 * 86_400;
        let before = self.downloads.len();
        self.downloads.retain(|d| !(d.status == Status::Done && d.finished.is_some_and(|f| f < cutoff)));
        if self.downloads.len() != before {
            self.changed();
        }
    }

    // -- settings ---------------------------------------------------------

    /// Change settings, save them and apply what changed.
    pub fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        change(&mut self.settings);
        if self.settings == before {
            return;
        }
        self.save_settings();
        self.apply_settings(&before);
    }

    fn apply_settings(&mut self, before: &Settings) {
        let s = &self.settings;
        if s.appearance() != before.appearance() {
            self.apply_theme();
        }
        self.global.set_rate(self.settings.speed_limit());
        self.clipboard_on.store(self.settings.clipboard_monitor, Ordering::Relaxed);
        let net = net_config(&self.settings);
        if net != self.net {
            self.agent = http::agent(&net);
            self.net = net;
        }
        if self.settings.forget_after_days != before.forget_after_days {
            self.forget_old();
        }
        self.schedule_open = None;
        self.schedule();
    }

    /// The color scheme for the settings and the system's mode.
    pub fn apply_theme(&mut self) {
        let a = self.settings.appearance();
        let name = a.scheme_name(self.system_dark).to_string();
        let scheme = schemes::get_scheme(&orchard_theme::dirs::schemes_dir(), &name);
        self.theme = Theme::new(&a, scheme);
    }

    pub fn set_system_dark(&mut self, dark: bool) {
        if self.system_dark != dark {
            self.system_dark = dark;
            self.apply_theme();
        }
    }

    /// ⌘+ / ⌘- / ⌘0.
    pub fn zoom(&mut self, step: i64) {
        let (lo, hi) = schemes::FONT_SIZE_RANGE;
        self.update_settings(|s| s.font_size = if step == 0 { settings::DEFAULT_FONT_SIZE } else { (s.font_size + step).clamp(lo, hi) });
    }

    pub fn cycle_traffic(&mut self) {
        self.update_settings(|s| s.traffic = s.traffic.next());
        let limit = self.settings.speed_limit();
        let what = if limit == 0 { "no limit".to_string() } else { format::speed(limit as f64) };
        self.toast(format!("Speed: {} · {what}", self.settings.traffic.label()), false);
    }

    // -- frame loop -------------------------------------------------------

    /// Run what workers and downloads sent. Returns whether anything changed.
    pub fn process(&mut self) -> bool {
        let mut changed = false;
        for done in self.tasks.take() {
            done(self);
            changed = true;
        }
        // A folder picker's listing, places and bookmarks.
        if let Some(p) = self.modal.as_mut().and_then(Modal::picker) {
            changed |= p.poll();
        }
        changed
    }

    /// Animation step: speeds, the toast's timeout, the schedule, saving.
    pub fn tick(&mut self, now: Instant) {
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.toast = None;
        }
        if now.duration_since(self.last_sample) >= Duration::from_millis(1000) {
            self.last_sample = now;
            let mut total = 0.0;
            for d in &mut self.downloads {
                if let Some(r) = &mut d.run {
                    total += r.sample(now);
                }
                if d.run.is_some() || !d.history.is_empty() {
                    d.push_history();
                }
            }
            if self.history.len() >= download::HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(total as f32);
            if self.downloads.iter().any(|d| d.run.is_some()) {
                self.dirty = true;
            }
            self.check_schedule();
        }
        if let Some(p) = &self.pending
            && now >= p.at
        {
            let action = p.action;
            self.pending = None;
            self.when_done_now(action);
        }
        if let Some(Modal::Add(a)) = &mut self.modal
            && a.probe_due.is_some_and(|t| now >= t)
        {
            a.probe_due = None;
            ui::add::probe(self);
        }
        if now.duration_since(self.last_save) > Duration::from_secs(5) {
            self.save();
        }
    }

    /// How soon the window wants the next frame: `None` = only on events.
    pub fn next_frame(&self, now: Instant) -> Option<Instant> {
        let toast = self.toast.as_ref().map(|t| t.until);
        let running = self.downloads.iter().any(|d| d.run.is_some()).then(|| now + Duration::from_millis(500));
        let pending = self.pending.as_ref().map(|_| now + Duration::from_millis(500));
        let probe = match &self.modal {
            Some(Modal::Add(a)) => a.probe_due,
            _ => None,
        };
        // The schedule and the clean-up need a look now and then.
        let idle = (self.settings.schedule_enabled || self.dirty).then(|| now + Duration::from_secs(10));
        [toast, running, pending, probe, idle].into_iter().flatten().min()
    }

    pub fn resized(&mut self, cols: u16, rows: u16, cell_px: (f32, f32)) {
        self.cols = cols;
        self.rows = rows;
        self.cell_px = cell_px;
        self.scroll_to_cursor();
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        let secs = if error { 7 } else { 3 };
        self.toast = Some(Toast { text: text.into(), error, until: Instant::now() + Duration::from_secs(secs) });
    }

    /// The window title: overall progress while downloading.
    pub fn title(&self) -> String {
        let running: Vec<&Download> = self.downloads.iter().filter(|d| d.status == Status::Running).collect();
        let (got, total) = running.iter().fold((0u64, 0u64), |(g, t), d| match d.size {
            Some(s) => (g + d.received_now(), t + s),
            None => (g, t),
        });
        if running.is_empty() || total == 0 {
            "Orchard Download Manager".into()
        } else {
            format!("{} % · Orchard Download Manager", got * 100 / total)
        }
    }

    pub fn total_speed(&self) -> f64 {
        self.downloads.iter().map(Download::speed).sum()
    }

    // -- the list -----------------------------------------------------------

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.downloads.iter().position(|d| d.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&Download> {
        self.downloads.iter().find(|d| d.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Download> {
        self.downloads.iter_mut().find(|d| d.id == id)
    }

    /// Indices of the downloads shown, in list order.
    pub fn view(&self) -> Vec<usize> {
        let words: Vec<String> = self
            .search
            .as_ref()
            .map(|s| s.field.text.to_lowercase().split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        self.downloads
            .iter()
            .enumerate()
            .filter(|(_, d)| self.tab.shows(d.status))
            .filter(|(_, d)| self.category.is_none_or(|c| d.category() == c))
            .filter(|(_, d)| {
                words.is_empty() || {
                    let hay = format!("{} {}", d.name, d.url).to_lowercase();
                    words.iter().all(|w| hay.contains(w.as_str()))
                }
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Downloads per tab, for the tab strip.
    pub fn tab_count(&self, tab: Tab) -> usize {
        self.downloads.iter().filter(|d| tab.shows(d.status)).count()
    }

    pub fn list_height(&self) -> usize {
        ui::layout(self).list.rows as usize
    }

    /// The cursor's row in the view.
    pub fn cursor_row(&self, view: &[usize]) -> Option<usize> {
        let id = self.cursor?;
        view.iter().position(|&i| self.downloads[i].id == id)
    }

    pub fn scroll_to_cursor(&mut self) {
        let view = self.view();
        let h = self.list_height().max(1);
        if let Some(c) = self.cursor_row(&view) {
            if c < self.list_top {
                self.list_top = c;
            } else if c >= self.list_top + h {
                self.list_top = c + 1 - h;
            }
        } else if let Some(&first) = view.first() {
            self.cursor = Some(self.downloads[first].id);
            self.list_top = 0;
        }
        self.list_top = self.list_top.min(view.len().saturating_sub(h));
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let view = self.view();
        if view.is_empty() {
            return;
        }
        let row = match self.cursor_row(&view) {
            Some(r) => (r as isize + delta).clamp(0, view.len() as isize - 1) as usize,
            None => 0,
        };
        self.cursor = Some(self.downloads[view[row]].id);
        self.scroll_to_cursor();
    }

    /// The downloads an action applies to: the marked ones, else the selected one.
    pub fn selection(&self) -> Vec<u64> {
        if !self.marked.is_empty() {
            return self.downloads.iter().filter(|d| self.marked.contains(&d.id)).map(|d| d.id).collect();
        }
        let view = self.view();
        self.cursor.filter(|_| self.cursor_row(&view).is_some()).into_iter().collect()
    }

    pub fn selected(&self) -> Option<&Download> {
        let view = self.view();
        self.cursor_row(&view).map(|r| &self.downloads[view[r]])
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.marked.clear();
        self.list_top = 0;
        self.scroll_to_cursor();
    }

    pub fn cycle_category(&mut self) {
        let all = Category::ALL;
        self.category = match self.category {
            None => Some(all[0]),
            Some(c) => all.iter().position(|&x| x == c).and_then(|i| all.get(i + 1)).copied(),
        };
        self.marked.clear();
        self.scroll_to_cursor();
    }

    fn toggle_mark(&mut self) {
        if let Some(id) = self.cursor.filter(|_| self.selected().is_some())
            && !self.marked.remove(&id)
        {
            self.marked.insert(id);
        }
        self.move_cursor(1);
    }

    fn mark_all(&mut self) {
        let view: HashSet<u64> = self.view().into_iter().map(|i| self.downloads[i].id).collect();
        self.marked = if self.marked == view { HashSet::new() } else { view };
    }

    /// Move the selection up or down the queue (list order).
    fn move_in_queue(&mut self, delta: isize) {
        let view = self.view();
        let Some(row) = self.cursor_row(&view) else { return };
        let to = row as isize + delta;
        if to < 0 || to >= view.len() as isize {
            return;
        }
        self.downloads.swap(view[row], view[to as usize]);
        self.changed();
        self.scroll_to_cursor();
        self.schedule();
    }

    // -- adding -------------------------------------------------------------

    /// Add downloads for `links`; returns how many were new.
    pub fn add_links(&mut self, links: &[String], opts: AddOptions) -> usize {
        let mut added = 0;
        for link in links {
            let mut d = Download::new(self.next_id, link.clone());
            self.next_id += 1;
            if links.len() == 1
                && let Some(n) = opts.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
            {
                d.name = http::sanitize_filename(n);
                d.name_fixed = true;
            }
            d.dir = opts.dir.clone();
            d.checksum = opts.checksum.clone();
            d.headers = opts.headers.get(link).cloned().unwrap_or_default();
            d.status = if opts.start { Status::Queued } else { Status::Paused };
            self.cursor = Some(d.id);
            self.downloads.push(d);
            added += 1;
        }
        if added > 0 {
            if let Some(dir) = &opts.dir {
                self.update_settings(|s| s.last_folder = Some(dir.clone()));
            }
            self.changed();
            if !self.tab.shows(if opts.start { Status::Queued } else { Status::Paused }) {
                self.tab = Tab::All;
            }
            self.scroll_to_cursor();
            self.schedule();
            self.save();
        }
        added
    }

    /// Links copied to the clipboard: offer the downloadable ones.
    pub fn clipboard_changed(&mut self, text: &str) {
        let known: HashSet<&str> = self.downloads.iter().map(|d| d.url.as_str()).collect();
        let links: Vec<String> =
            crate::links::catchable(text, |e| self.settings.catches(e)).into_iter().filter(|l| !known.contains(l.as_str())).collect();
        if links.is_empty() {
            return;
        }
        match &mut self.modal {
            None => {
                ui::add::open(self, &links.join(" "));
                self.attention = true;
            }
            Some(Modal::Add(a)) if a.links.text.trim().is_empty() => {
                a.links.set_text(&links.join(" "));
                ui::add::links_changed(a);
            }
            _ => {}
        }
    }

    /// Links from another start of the app (or nothing: just come forward).
    pub fn external_links(&mut self, links: Vec<String>) {
        self.handoff(crate::ipc::Handoff { links, ..Default::default() });
    }

    /// Links from another start or from the browser: the Add dialog, or
    /// straight into the list (the browser, if the settings say so).
    pub fn handoff(&mut self, h: crate::ipc::Handoff) {
        let mut items: Vec<crate::ipc::Link> = crate::links::find(&h.links.join(" ")).into_iter().map(crate::ipc::Link::plain).collect();
        items.extend(h.items.into_iter().filter(|l| l.url.starts_with("http://") || l.url.starts_with("https://")));
        let ask = !h.browser || self.settings.browser_ask;
        if ask {
            self.raise = true;
        }
        if items.is_empty() {
            return;
        }
        if ask && self.modal.is_none() {
            ui::add::open_links(self, &items);
            return;
        }
        let urls: Vec<String> = items.iter().map(|l| l.url.clone()).collect();
        let name = match items.as_slice() {
            [one] => one.name.clone(),
            _ => None,
        };
        let headers = items.into_iter().filter(|l| !l.headers.is_empty()).map(|l| (l.url, l.headers)).collect();
        let n = self.add_links(&urls, AddOptions { start: self.settings.start_immediately, name, headers, ..AddOptions::default() });
        self.toast(format!("Added {}", format::count(n, "download", "downloads")), false);
        self.attention = true;
    }

    // -- running ------------------------------------------------------------

    /// Start downloads by hand: right away while there is room, else queued.
    pub fn start(&mut self, ids: &[u64]) {
        let mut room = self.settings.active_limit().saturating_sub(self.running());
        for &id in ids {
            let Some(d) = self.get_mut(id) else { continue };
            match d.status {
                Status::Paused | Status::Failed | Status::Queued => {
                    d.status = Status::Queued;
                    d.error = None;
                    if room > 0 {
                        room -= 1;
                        self.launch(id);
                    }
                }
                Status::Running => {
                    // Stopping: carry on instead.
                    if let Some(r) = &mut d.run
                        && r.stopping
                    {
                        r.then = Then::Restart;
                    }
                }
                Status::Done => {}
            }
        }
        self.changed();
        self.schedule();
    }

    pub fn pause(&mut self, ids: &[u64]) {
        for &id in ids {
            let Some(d) = self.get_mut(id) else { continue };
            match d.status {
                Status::Running => {
                    if let Some(r) = &mut d.run {
                        r.shared.stop();
                        r.stopping = true;
                        r.then = Then::Pause;
                    }
                }
                Status::Queued => d.status = Status::Paused,
                _ => {}
            }
        }
        self.changed();
        self.schedule();
    }

    /// Space / Enter on a download: pause what runs, start what doesn't.
    pub fn toggle(&mut self, ids: &[u64]) {
        let any_running = ids.iter().any(|&id| self.get(id).is_some_and(|d| d.is_active()));
        if any_running { self.pause(ids) } else { self.start(ids) }
    }

    pub fn start_all(&mut self) {
        let ids: Vec<u64> = self.downloads.iter().filter(|d| matches!(d.status, Status::Paused | Status::Failed)).map(|d| d.id).collect();
        for &id in &ids {
            if let Some(d) = self.get_mut(id) {
                d.status = Status::Queued;
                d.error = None;
            }
        }
        self.changed();
        self.schedule_open = None;
        self.schedule();
    }

    pub fn pause_all(&mut self) {
        let ids: Vec<u64> = self.downloads.iter().filter(|d| d.is_active()).map(|d| d.id).collect();
        self.pause(&ids);
    }

    /// Download again from the start (a finished file stays until replaced).
    pub fn restart(&mut self, ids: &[u64]) {
        for &id in ids {
            let Some(d) = self.get_mut(id) else { continue };
            if let Some(r) = &mut d.run {
                r.shared.stop();
                r.stopping = true;
                r.then = Then::Restart;
                d.segments.clear();
                d.received = 0;
                continue;
            }
            if let Some(t) = &d.target
                && d.status != Status::Done
            {
                let _ = std::fs::remove_file(engine::part_path(t));
            }
            d.segments.clear();
            d.received = 0;
            d.target = None;
            d.size = None;
            d.verified = None;
            d.finished = None;
            d.error = None;
            d.status = Status::Queued;
        }
        self.changed();
        self.schedule();
    }

    pub fn running(&self) -> usize {
        self.downloads.iter().filter(|d| d.status == Status::Running).count()
    }

    /// Whether the schedule lets queued downloads start now.
    pub fn schedule_allows(&self) -> bool {
        let s = &self.settings;
        if !s.schedule_enabled {
            return true;
        }
        use chrono::{Datelike, Timelike};
        let now = chrono::Local::now();
        in_window(s, now.weekday().num_days_from_monday() as usize, now.hour() * 60 + now.minute())
    }

    /// The schedule opened or closed since the last look: start what waits,
    /// or put what runs back in the queue.
    fn check_schedule(&mut self) {
        if !self.settings.schedule_enabled {
            self.schedule_open = None;
            return;
        }
        let open = self.schedule_allows();
        if self.schedule_open == Some(open) {
            return;
        }
        let was = self.schedule_open.replace(open);
        if open {
            self.schedule();
        } else if was == Some(true) {
            for d in &mut self.downloads {
                if let Some(r) = &mut d.run {
                    r.shared.stop();
                    r.stopping = true;
                    r.then = Then::Queue;
                }
            }
            self.toast("The schedule paused the downloads until it starts again", false);
        }
    }

    /// Start queued downloads while there is room (and the schedule allows).
    pub fn schedule(&mut self) {
        if self.schedule_allows() {
            let mut room = self.settings.active_limit().saturating_sub(self.running());
            let next: Vec<u64> = self.downloads.iter().filter(|d| d.status == Status::Queued).map(|d| d.id).collect();
            for id in next {
                if room == 0 {
                    break;
                }
                room -= 1;
                self.launch(id);
            }
        }
        let busy = self.running() > 0;
        if busy && self.settings.prevent_sleep {
            if self.awake.is_none() && self.persist {
                self.awake = Some(system::Awake::hold());
            }
        } else {
            self.awake = None;
        }
    }

    fn spec_for(&self, d: &Download) -> Spec {
        let s = &self.settings;
        Spec {
            url: d.url.clone(),
            target: d.target.clone().filter(|_| !d.segments.is_empty()),
            size: d.size,
            segments: d.segments.clone(),
            validator: d.validator.clone(),
            headers: d.headers.clone(),
            connections: d.connections.unwrap_or(s.connections).max(1) as usize,
            min_segment: s.min_segment_kib.max(64) as u64 * 1024,
            retries: s.retries,
            retry_delay: Duration::from_secs(s.retry_delay_secs.max(1) as u64),
            stall: Duration::from_secs(s.timeout_secs.max(5) as u64),
            overwrite: s.on_conflict == settings::Conflict::Overwrite,
        }
    }

    fn launch(&mut self, id: u64) {
        let agent = match &self.agent {
            Ok(a) => a.clone(),
            Err(e) => {
                let e = e.clone();
                if let Some(d) = self.get_mut(id) {
                    d.status = Status::Failed;
                    d.error = Some(format!("{e} — check the proxy in Settings › Network"));
                }
                return;
            }
        };
        let Some(i) = self.index_of(id) else { return };
        let spec = self.spec_for(&self.downloads[i]);
        let d = &mut self.downloads[i];
        let shared = Shared::new(d.speed_limit.unwrap_or(0));
        shared.received.store(d.received, Ordering::Relaxed);
        let tasks = self.tasks.clone();
        let report: Reporter = Arc::new(move |ev| tasks.post(move |app: &mut App| app.engine_event(id, ev)));
        d.run = Some(Run::new(shared.clone()));
        d.status = Status::Running;
        d.error = None;
        engine::start(spec, shared, self.global.clone(), agent, report);
    }

    /// The path a download saves to, once the server said its name.
    fn choose_target(&self, id: u64, probe: &http::Probe) -> PathBuf {
        let d = self.get(id);
        let name = match d {
            Some(d) if d.name_fixed => d.name.clone(),
            _ => probe.name.clone(),
        };
        let dir = d.and_then(|d| d.dir.clone()).unwrap_or_else(|| self.settings.dir_for(Category::of(&name)));
        let path = dir.join(&name);
        if self.settings.on_conflict == settings::Conflict::Overwrite {
            return path;
        }
        let taken: Vec<&Path> = self.downloads.iter().filter(|o| o.id != id && o.status != Status::Done).filter_map(|o| o.file()).collect();
        engine::unique_path(&path, |p| taken.contains(&p))
    }

    pub fn engine_event(&mut self, id: u64, ev: Event) {
        self.changed();
        match ev {
            Event::Probed { probe, reply } => {
                let target = self.choose_target(id, &probe);
                if let Some(d) = self.get_mut(id) {
                    d.size = probe.size;
                    d.resumable = Some(probe.resumable);
                    d.name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    d.target = Some(target.clone());
                }
                let _ = reply.send(Some(target));
            }
            Event::Running { probe, target, restarted } => {
                let Some(d) = self.get_mut(id) else { return };
                d.size = probe.size;
                d.resumable = Some(probe.resumable);
                d.validator = probe.validator;
                d.name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                d.target = Some(target);
                if let Some(why) = restarted {
                    let name = d.name.clone();
                    self.toast(format!("Restarted {name}: {why}"), false);
                }
            }
            Event::Paused => {
                let Some(d) = self.get_mut(id) else { return };
                d.sync_from_run();
                let then = d.run.take().map_or(Then::Pause, |r| r.then);
                d.status = match then {
                    Then::Pause => Status::Paused,
                    Then::Queue | Then::Restart => Status::Queued,
                    Then::Remove { .. } => Status::Paused,
                };
                if then == Then::Restart && d.received == 0 && d.segments.is_empty() {
                    d.target = None;
                }
                if let Then::Remove { delete_files } = then {
                    self.forget(&[id], delete_files);
                }
                self.schedule();
            }
            Event::Done(path) => {
                let Some(d) = self.get_mut(id) else { return };
                d.sync_from_run();
                d.run = None;
                d.status = Status::Done;
                d.finished = Some(format::now_unix());
                d.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                d.received = d.size.unwrap_or(d.received);
                if d.size.is_none() {
                    d.size = Some(d.received);
                }
                d.target = Some(path.clone());
                d.segments.clear();
                let name = d.name.clone();
                let expected = d.checksum.clone().filter(|_| self.settings.verify_checksums);
                if self.settings.notify_done {
                    self.toast(format!("Downloaded {name}"), false);
                    self.attention = true;
                }
                if let Some(exp) = expected {
                    self.verify(id, path, exp);
                }
                self.schedule();
                if self.settings.when_done != WhenDone::Nothing && !self.downloads.iter().any(Download::is_active) {
                    self.pending = Some(Pending { action: self.settings.when_done, at: Instant::now() + WHEN_DONE_DELAY });
                    self.attention = true;
                }
            }
            Event::Failed(e) => {
                let Some(d) = self.get_mut(id) else { return };
                d.sync_from_run();
                let then = d.run.take().map(|r| r.then);
                if let Some(Then::Remove { delete_files }) = then {
                    self.forget(&[id], delete_files);
                } else {
                    d.status = Status::Failed;
                    let name = d.name.clone();
                    d.error = Some(e.clone());
                    self.toast(format!("Can't download {name}: {e}"), true);
                }
                self.schedule();
            }
        }
    }

    /// Compare a finished file with its expected checksum (in the background).
    fn verify(&mut self, id: u64, path: PathBuf, exp: Expected) {
        self.tasks.spawn(
            move || crate::checksum::compute(&path, &[exp.algo], |_| {}).map(|v| v[0].1 == exp.hex),
            move |app, r| {
                let Some(d) = app.get_mut(id) else { return };
                let name = d.name.clone();
                match r {
                    Ok(ok) => {
                        d.verified = Some(ok);
                        app.changed();
                        if ok {
                            app.toast(format!("{name}: checksum matches"), false);
                        } else {
                            app.toast(format!("{name} doesn't match its checksum — the file may be damaged; download it again with R"), true);
                        }
                    }
                    Err(e) => app.toast(format!("Can't check {name}: {}", format::io_reason(&e)), true),
                }
            },
        );
    }

    /// The countdown after the last download ran out.
    fn when_done_now(&mut self, action: WhenDone) {
        if self.downloads.iter().any(Download::is_active) {
            return; // something new came in meanwhile
        }
        match action {
            WhenDone::Nothing => {}
            WhenDone::Quit => self.quit = true,
            WhenDone::Sleep => {
                if let Err(e) = system::sleep_now() {
                    self.toast(format!("Can't put the computer to sleep: {e}"), true);
                }
            }
        }
    }

    pub fn cancel_pending(&mut self) {
        if self.pending.take().is_some() {
            self.toast("Canceled", false);
        }
    }

    // -- removing -------------------------------------------------------------

    /// Ask (if the settings say so), then remove.
    pub fn ask_remove(&mut self, delete_files: bool) {
        let ids = self.selection();
        if ids.is_empty() {
            return;
        }
        if self.settings.confirm_remove || delete_files {
            self.modal = Some(Modal::Remove(dialogs::Remove::new(self, ids, delete_files)));
        } else {
            self.remove(&ids, false);
        }
    }

    /// Remove downloads from the list; unfinished ones lose their partial
    /// file, `delete_files` deletes finished files too.
    pub fn remove(&mut self, ids: &[u64], delete_files: bool) {
        let mut now = Vec::new();
        for &id in ids {
            let Some(d) = self.get_mut(id) else { continue };
            match &mut d.run {
                Some(r) => {
                    r.shared.stop();
                    r.stopping = true;
                    r.then = Then::Remove { delete_files };
                }
                None => now.push(id),
            }
        }
        let n = ids.len();
        self.forget(&now, delete_files);
        self.toast(
            if delete_files { format!("Deleted {}", format::count(n, "download", "downloads")) } else { format!("Removed {}", format::count(n, "download", "downloads")) },
            false,
        );
    }

    fn forget(&mut self, ids: &[u64], delete_files: bool) {
        let mut files = Vec::new();
        for &id in ids {
            let Some(i) = self.index_of(id) else { continue };
            let d = self.downloads.remove(i);
            self.marked.remove(&id);
            if let Some(t) = &d.target {
                if d.status != Status::Done {
                    files.push(engine::part_path(t));
                } else if delete_files {
                    files.push(t.clone());
                }
            }
        }
        if !files.is_empty() {
            self.tasks.spawn(
                move || files.iter().filter_map(|f| std::fs::remove_file(f).err().filter(|e| e.kind() != std::io::ErrorKind::NotFound)).count(),
                |app, failed| {
                    if failed > 0 {
                        app.toast(format!("Couldn't delete {} — they may be open in another app", format::count(failed, "file", "files")), true);
                    }
                },
            );
        }
        self.changed();
        self.scroll_to_cursor();
    }

    // -- files ----------------------------------------------------------------

    /// Open the selected finished file.
    pub fn open_file(&mut self) {
        let Some(d) = self.selected() else { return };
        let Some(path) = d.file().map(Path::to_path_buf).filter(|_| d.status == Status::Done) else {
            self.toast("This download isn't finished — open it when it's done, or show it with F4", true);
            return;
        };
        if !path.exists() {
            self.toast(format!("Can't open {}: it was moved or deleted", path.display()), true);
            return;
        }
        if let Err(e) = system::open(&path) {
            self.toast(format!("Can't open {}: {e}", path.display()), true);
        }
    }

    /// Show the selected download's file (or its folder) in the file manager.
    pub fn reveal(&mut self) {
        let Some(d) = self.selected() else { return };
        let path = match (d.status, d.file()) {
            (Status::Done, Some(p)) => p.to_path_buf(),
            (_, Some(p)) if engine::part_path(p).exists() => engine::part_path(p),
            (_, Some(p)) => p.parent().map(Path::to_path_buf).unwrap_or_default(),
            (_, None) => d.dir.clone().unwrap_or_else(|| self.settings.dir_for(d.category())),
        };
        let shown = if path.exists() { path } else { path.parent().map(Path::to_path_buf).unwrap_or(path) };
        if let Err(e) = system::reveal(&shown) {
            self.toast(format!("Can't show {}: {e}", shown.display()), true);
        }
    }

    pub fn copy_link(&mut self) {
        let Some(url) = self.selected().map(|d| d.url.clone()) else { return };
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(url)) {
            Ok(()) => self.toast("Copied the link", false),
            Err(e) => self.toast(format!("Can't copy the link: {e}"), true),
        }
    }

    /// Apply the properties dialog's changes to a download.
    pub fn edit(&mut self, id: u64, edit: dialogs::Edit) {
        let Some(d) = self.get_mut(id) else { return };
        let mut restart = false;
        if let Some(url) = edit.url
            && url != d.url
        {
            d.url = url;
            // A new address for the same file (an expired link): go on with it.
            restart = d.run.is_some();
            if d.status == Status::Failed {
                d.status = Status::Paused;
                d.error = None;
            }
        }
        if let Some(name) = edit.name {
            d.name = name;
            d.name_fixed = true;
            d.target = None;
        }
        if let Some(dir) = edit.dir {
            d.dir = dir;
            d.target = None;
        }
        d.connections = edit.connections;
        d.speed_limit = edit.speed_limit;
        if let Some(r) = &d.run {
            r.shared.limit.set_rate(d.speed_limit.unwrap_or(0));
        }
        d.checksum = edit.checksum;
        if restart && let Some(r) = &mut d.run {
            r.shared.stop();
            r.stopping = true;
            r.then = Then::Restart;
        }
        self.changed();
    }

    // -- input --------------------------------------------------------------

    pub fn handle_key(&mut self, ev: &KeyEvent) {
        if self.modal.is_some() {
            ui::modal_key(self, ev);
            return;
        }
        if self.search_key(ev) {
            return;
        }
        let m = ev.mods;
        if m.primary() {
            match &ev.key {
                Key::Char('n') => ui::add::open(self, ""),
                Key::Char('v') => {
                    let text = orchard_kit::clipboard::text().unwrap_or_default();
                    ui::add::open(self, &crate::links::find(&text).join(" "));
                }
                Key::Char('c') => self.copy_link(),
                Key::Char('f') => self.start_search(),
                Key::Char('a') => self.mark_all(),
                Key::Char(',') => ui::settings::open(self, None),
                Key::Char('q') => self.quit = true,
                Key::Char('o') => self.open_file(),
                Key::Char('=' | '+') => self.zoom(1),
                Key::Char('-') => self.zoom(-1),
                Key::Char('0') => self.zoom(0),
                Key::Named(Named::Up) => self.move_in_queue(-1),
                Key::Named(Named::Down) => self.move_in_queue(1),
                Key::Named(Named::Backspace) => self.ask_remove(false),
                _ => {}
            }
            return;
        }
        let page = self.list_height().max(2) as isize - 1;
        match &ev.key {
            Key::Named(Named::F(5)) if m.shift => self.start_all(),
            Key::Named(Named::F(6)) if m.shift => self.pause_all(),
            Key::Named(Named::F(8)) if m.shift => self.ask_remove(true),
            Key::Named(Named::F(n)) => self.fkey(*n),
            Key::Named(Named::Up) if m.alt => self.move_in_queue(-1),
            Key::Named(Named::Down) if m.alt => self.move_in_queue(1),
            Key::Named(Named::Up) => self.move_cursor(-1),
            Key::Named(Named::Down) => self.move_cursor(1),
            Key::Named(Named::PageUp) => self.move_cursor(-page),
            Key::Named(Named::PageDown) => self.move_cursor(page),
            Key::Named(Named::Home) => self.move_cursor(isize::MIN / 2),
            Key::Named(Named::End) => self.move_cursor(isize::MAX / 2),
            Key::Named(Named::Tab) => {
                let i = Tab::ALL.iter().position(|&t| t == self.tab).unwrap_or(0);
                let n = Tab::ALL.len();
                self.set_tab(Tab::ALL[if m.shift { (i + n - 1) % n } else { (i + 1) % n }]);
            }
            Key::Named(Named::Enter) => match self.selected().map(|d| d.status) {
                Some(Status::Done) => self.open_file(),
                Some(_) => self.toggle(&self.selection()),
                None => {}
            },
            Key::Named(Named::Escape) => {
                if self.pending.is_some() {
                    self.cancel_pending();
                } else if !self.marked.is_empty() {
                    self.marked.clear();
                } else if self.search.is_some() {
                    self.search = None;
                    self.scroll_to_cursor();
                }
            }
            Key::Named(Named::Delete) => self.ask_remove(m.shift),
            Key::Named(Named::Insert) => self.toggle_mark(),
            _ => match ev.typed() {
                Some(" ") => self.toggle(&self.selection()),
                Some("i" | "I") => self.update_settings(|s| s.show_details = !s.show_details),
                Some("c" | "C") => self.cycle_category(),
                Some("e" | "E") => dialogs::open_properties(self),
                Some("h" | "H") => dialogs::open_checksum(self),
                Some("r" | "R") => self.restart(&self.selection()),
                Some("l" | "L") => self.copy_link(),
                Some("s" | "S") => self.start_all(),
                Some("p" | "P") => self.pause_all(),
                Some("+" | "=") => self.move_in_queue(-1),
                Some("-") => self.move_in_queue(1),
                Some("/") => self.start_search(),
                Some("?") => self.modal = Some(Modal::Help),
                Some("1") => self.set_tab(Tab::All),
                Some("2") => self.set_tab(Tab::Active),
                Some("3") => self.set_tab(Tab::Paused),
                Some("4") => self.set_tab(Tab::Done),
                Some("5") => self.set_tab(Tab::Failed),
                _ => {}
            },
        }
    }

    /// The F-key bar.
    pub fn fkey(&mut self, n: u8) {
        match n {
            1 => self.modal = Some(Modal::Help),
            2 => ui::add::open(self, ""),
            3 => self.open_file(),
            4 => self.reveal(),
            5 => self.start(&self.selection()),
            6 => self.pause(&self.selection()),
            7 => self.cycle_traffic(),
            8 => self.ask_remove(false),
            9 => ui::settings::open(self, None),
            10 => self.quit = true,
            _ => {}
        }
    }

    pub fn start_search(&mut self) {
        self.search.get_or_insert_with(Search::default).editing = true;
        self.scroll_to_cursor();
    }

    /// Keys while typing in the search; returns whether the key was taken.
    fn search_key(&mut self, ev: &KeyEvent) -> bool {
        let Some(s) = &mut self.search else { return false };
        if !s.editing {
            return false;
        }
        match &ev.key {
            Key::Named(Named::Escape) => {
                self.search = None;
                self.scroll_to_cursor();
            }
            Key::Named(Named::Enter | Named::Tab) => s.editing = false,
            Key::Named(Named::Up | Named::Down | Named::PageUp | Named::PageDown | Named::F(_)) => return false,
            _ => {
                if s.field.handle(ev) {
                    self.list_top = 0;
                    self.scroll_to_cursor();
                } else {
                    return false;
                }
            }
        }
        true
    }

    pub fn handle_mouse(&mut self, ev: &MouseEvent) {
        if self.modal.is_some() {
            ui::modal_mouse(self, ev);
            return;
        }
        let l = ui::layout(self);
        let hit = |r: orchard_grid::CellRect| orchard_kit::widgets::contains(r, ev.col, ev.row);
        match ev.kind {
            MouseKind::Down => {
                if hit(l.list) {
                    let view = self.view();
                    let row = self.list_top + (ev.row - l.list.row) as usize;
                    if let Some(&i) = view.get(row) {
                        let id = self.downloads[i].id;
                        self.cursor = Some(id);
                        if ev.double {
                            match self.downloads[i].status {
                                Status::Done => self.open_file(),
                                _ => self.toggle(&[id]),
                            }
                        }
                    }
                } else if let Some(t) = ui::tab_at(self, &l, ev.col, ev.row) {
                    self.set_tab(t);
                } else if l.category.is_some_and(hit) {
                    self.cycle_category();
                } else if hit(l.footer) {
                    if let Some((_, i)) = orchard_kit::widgets::key_bar_items(l.footer, ui::FKEYS).into_iter().find(|(r, _)| orchard_kit::widgets::contains(*r, ev.col, ev.row)) {
                        self.fkey(i as u8 + 1);
                    }
                } else if l.status.is_some_and(hit) {
                    self.cycle_traffic();
                }
            }
            MouseKind::Scroll(dy) if hit(l.list) => {
                let h = l.list.rows as usize;
                let max = self.view().len().saturating_sub(h);
                self.list_top = (self.list_top as f32 - dy * 3.0).round().clamp(0.0, max as f32) as usize;
            }
            _ => {}
        }
    }
}

/// The client settings from the app's.
fn net_config(s: &Settings) -> NetConfig {
    NetConfig {
        user_agent: s.user_agent.clone(),
        proxy: match s.proxy_mode {
            ProxyMode::System => Some(String::new()),
            ProxyMode::None => None,
            ProxyMode::Manual if s.proxy.trim().is_empty() => None,
            ProxyMode::Manual => Some(s.proxy.trim().to_string()),
        },
        timeout: Duration::from_secs(s.timeout_secs.max(5) as u64),
    }
}

/// Whether minute `now` of weekday `day` (Monday 0) is in the schedule's
/// window. A window past midnight belongs to the day it starts on; no stop
/// time: open until midnight.
pub fn in_window(s: &Settings, day: usize, now: u32) -> bool {
    let Some(start) = settings::parse_hhmm(&s.schedule_start) else { return true };
    let stop = settings::parse_hhmm(&s.schedule_stop).unwrap_or(24 * 60);
    let on = |d: usize| s.schedule_days[d % 7];
    if start < stop {
        on(day) && (start..stop).contains(&now)
    } else if start > stop {
        (on(day) && now >= start) || (on((day + 6) % 7) && now < stop)
    } else {
        on(day)
    }
}

#[cfg(test)]
mod tests;
