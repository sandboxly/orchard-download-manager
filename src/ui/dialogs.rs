//! The smaller dialogs: removing downloads, a download's properties, its
//! checksums, and the keys.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use orchard_grid::{CellRect, Frame, Style};
use orchard_kit::input::{KeyEvent, MouseEvent, Named};
use orchard_kit::widgets::{BoxFrame, DrawCtx, TextField, put, width};

use crate::app::{App, Modal};
use crate::checksum::{self, Algo, Expected};
use crate::download::Status;
use crate::format;
use crate::ui::add::expand_tilde;
use crate::ui::form::{self, Act, Ctl, Form, Kind, TextKind};
use crate::ui::picker::{FolderPicker, Outcome};

// ---------------------------------------------------------------------------
// Remove
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveId {
    Remove,
    Cancel,
}

pub struct Remove {
    pub ids: Vec<u64>,
    pub delete_files: bool,
    focus: Option<RemoveId>,
    lines: Vec<String>,
}

impl Remove {
    pub fn new(app: &App, ids: Vec<u64>, delete_files: bool) -> Remove {
        let ds: Vec<_> = ids.iter().filter_map(|&id| app.get(id)).collect();
        let n = ds.len();
        let what = if n == 1 { format!("“{}”", ds[0].name) } else { format::count(n, "download", "downloads") };
        let done: Vec<_> = ds.iter().filter(|d| d.status == Status::Done).collect();
        let unfinished = n - done.len();
        let mut lines = Vec::new();
        if delete_files {
            let bytes: u64 = done.iter().filter_map(|d| d.size).sum();
            lines.push(format!("Delete {what} and the files?"));
            if !done.is_empty() {
                lines.push(format!("{} ({}) go off the disk. You can't undo this.", format::count(done.len(), "finished file", "finished files"), format::size(bytes)));
            } else {
                lines.push("Partly downloaded files are deleted. You can't undo this.".into());
            }
        } else {
            lines.push(format!("Remove {what} from the list?"));
            match (done.is_empty(), unfinished) {
                (false, 0) => lines.push("The downloaded files stay where they are.".into()),
                (true, _) => lines.push("What was downloaded so far is deleted.".into()),
                (false, _) => lines.push("Finished files stay; partly downloaded ones are deleted.".into()),
            }
        }
        Remove { ids, delete_files, focus: Some(RemoveId::Remove), lines }
    }

    fn rects(app: &App) -> (CellRect, CellRect) {
        form::dialog_rects(crate::ui::screen(app), 72, 6)
    }

    fn layout(&self, app: &App) -> Form<RemoveId> {
        let (_, content) = Self::rects(app);
        let mut f = Form::default();
        for (i, l) in self.lines.iter().enumerate() {
            f.label(CellRect::new(content.col, content.row + i as u16, content.cols, 1), l.clone(), if i == 0 { TextKind::Label } else { TextKind::Muted });
        }
        let label = if self.delete_files { "Delete files" } else { "Remove from list" };
        let b = form::ButtonRow::new(&[label, "Cancel"]).rects(CellRect::new(content.col, content.row + 4, content.cols, 1));
        f.ctls.push(Ctl::new(RemoveId::Remove, b[0], Kind::Button(label.into())));
        f.ctls.push(Ctl::new(RemoveId::Cancel, b[1], Kind::Button("Cancel".into())));
        f
    }

    pub fn draw(&self, app: &App, f: &mut Frame, ctx: &DrawCtx) {
        let (outer, _) = Self::rects(app);
        form::dialog_box(f, outer, if self.delete_files { "Delete files" } else { "Remove downloads" }, ctx);
        self.layout(app).draw(f, self.focus, ctx);
    }

    pub fn key(&mut self, app: &mut App, ev: &KeyEvent) -> bool {
        let act = self.layout(app).handle_key(&mut self.focus, ev, None);
        self.act(app, act)
    }

    pub fn mouse(&mut self, app: &mut App, ev: &MouseEvent) -> bool {
        let act = self.layout(app).handle_mouse(&mut self.focus, ev);
        self.act(app, act)
    }

    fn act(&mut self, app: &mut App, act: Act<RemoveId>) -> bool {
        match act {
            Act::Press(RemoveId::Remove) => {
                let ids = std::mem::take(&mut self.ids);
                app.remove(&ids, self.delete_files);
                app.marked.clear();
                false
            }
            Act::Press(RemoveId::Cancel) | Act::Escape => false,
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Properties (E)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropId {
    Url,
    Name,
    Dir,
    Choose,
    Connections,
    Limit,
    Checksum,
    Save,
    Cancel,
}

/// What the properties dialog changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edit {
    pub url: Option<String>,
    pub name: Option<String>,
    /// `Some(None)`: by type.
    pub dir: Option<Option<PathBuf>>,
    pub connections: Option<u32>,
    pub speed_limit: Option<u64>,
    pub checksum: Option<Expected>,
}

pub struct Properties {
    id: u64,
    url: TextField,
    name: TextField,
    dir: TextField,
    connections: Option<u32>,
    /// KiB/s.
    limit: Option<u32>,
    checksum: TextField,
    /// Name and folder can change only before anything was downloaded.
    can_move: bool,
    focus: Option<PropId>,
    error: Option<String>,
    picker: Option<FolderPicker>,
}

pub fn open_properties(app: &mut App) {
    let Some(d) = app.selected() else { return };
    let can_move = d.run.is_none() && d.received_now() == 0 && d.status != Status::Done;
    let dir = d.dir.as_deref().map(crate::ui::home_relative).unwrap_or_default();
    let p = Properties {
        id: d.id,
        url: TextField::new(d.url.clone()),
        name: TextField::new(if d.name_fixed || !can_move { d.name.clone() } else { String::new() }),
        dir: TextField::new(if can_move { dir } else { d.file().and_then(|f| f.parent()).map(crate::ui::home_relative).unwrap_or(dir) }),
        connections: d.connections,
        limit: d.speed_limit.map(|b| (b / 1024) as u32),
        checksum: TextField::new(d.checksum.as_ref().map(|c| format!("{}:{}", c.algo.label().to_lowercase().replace('-', ""), c.hex)).unwrap_or_default()),
        can_move,
        focus: Some(PropId::Url),
        error: None,
        picker: None,
    };
    app.modal = Some(Modal::Properties(p));
}

const LIMITS: &[u32] = &[32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536];

impl Properties {
    fn rects(app: &App) -> (CellRect, CellRect) {
        form::dialog_rects(crate::ui::screen(app), 92, 11)
    }

    fn layout(&self, app: &App) -> Form<PropId> {
        let (_, c) = Self::rects(app);
        let mut f = Form::default();
        let lw = 14;
        let (fx, fw) = (c.col + lw, c.cols.saturating_sub(lw));
        let row = |i: u16| c.row + i;
        let text = |f: &mut Form<PropId>, i: u16, label: &str, id: PropId, tf: &TextField, hint: &str, disabled: bool| {
            f.label(CellRect::new(c.col, row(i), lw, 1), label, if disabled { TextKind::Muted } else { TextKind::Label });
            f.ctls.push(Ctl::new(id, CellRect::new(fx, row(i), fw, 1), Kind::Text(tf.clone(), hint.into())).disabled(disabled));
        };
        text(&mut f, 0, "Address", PropId::Url, &self.url, "", false);
        let d = app.get(self.id);
        let server_name = d.map(|d| d.name.clone()).unwrap_or_default();
        text(&mut f, 1, "Name", PropId::Name, &self.name, &server_name, !self.can_move);
        let auto = d.map(|d| crate::ui::home_relative(&app.settings.dir_for(d.category()))).unwrap_or_default();
        text(&mut f, 2, "Folder", PropId::Dir, &self.dir, &auto, !self.can_move);
        if let Some(dir) = f.ctls.last_mut() {
            dir.rect.cols = fw.saturating_sub(13);
        }
        f.ctls.push(Ctl::new(PropId::Choose, CellRect::new(fx + fw.saturating_sub(12), row(2), 12, 1), Kind::Button("Choose…".into())).disabled(!self.can_move));
        text(&mut f, 3, "Checksum", PropId::Checksum, &self.checksum, "optional: SHA-256, SHA-1 or MD5", false);
        f.label(CellRect::new(c.col, row(5), lw, 1), "Connections", TextKind::Label);
        let conns = match self.connections {
            None => format!("default ({})", app.settings.connections),
            Some(n) => n.to_string(),
        };
        f.ctls.push(Ctl::new(PropId::Connections, CellRect::new(fx, row(5), 18, 1), Kind::Spin(conns)));
        f.label(CellRect::new(c.col, row(6), lw, 1), "Speed limit", TextKind::Label);
        let limit = self.limit.map_or("none".to_string(), |k| format::speed(k as f64 * 1024.0));
        f.ctls.push(Ctl::new(PropId::Limit, CellRect::new(fx, row(6), 18, 1), Kind::Spin(limit)));
        let note = match (&self.error, self.can_move) {
            (Some(e), _) => (e.clone(), TextKind::Error),
            (None, false) => ("Name and folder are fixed once started; a new address must be for the same file".to_string(), TextKind::Muted),
            (None, true) => ("Empty name: the server's. Empty folder: by file type.".to_string(), TextKind::Muted),
        };
        f.label(CellRect::new(c.col, row(8), c.cols, 1), note.0, note.1);
        let b = form::ButtonRow::new(&["Save", "Cancel"]).rects(CellRect::new(c.col, row(10), c.cols, 1));
        f.ctls.push(Ctl::new(PropId::Save, b[0], Kind::Button("Save".into())));
        f.ctls.push(Ctl::new(PropId::Cancel, b[1], Kind::Button("Cancel".into())));
        f
    }

    pub fn draw(&self, app: &App, f: &mut Frame, ctx: &DrawCtx) {
        let (outer, _) = Self::rects(app);
        form::dialog_box(f, outer, "Properties", ctx);
        self.layout(app).draw(f, self.focus, ctx);
        if let Some(p) = &self.picker {
            p.draw(f, crate::ui::screen(app), ctx);
        }
    }

    fn picked(&mut self, out: Outcome) {
        match out {
            Outcome::Keep => {}
            Outcome::Cancel => self.picker = None,
            Outcome::Chosen(p) => {
                self.dir.set_text(&crate::ui::home_relative(&p));
                self.picker = None;
                self.focus = Some(PropId::Save);
            }
        }
    }

    fn field(&mut self, id: PropId) -> Option<&mut TextField> {
        match id {
            PropId::Url => Some(&mut self.url),
            PropId::Name => Some(&mut self.name),
            PropId::Dir => Some(&mut self.dir),
            PropId::Checksum => Some(&mut self.checksum),
            _ => None,
        }
    }

    pub fn key(&mut self, app: &mut App, ev: &KeyEvent) -> bool {
        if let Some(p) = &mut self.picker {
            let out = p.key(ev, crate::ui::screen(app));
            self.picked(out);
            return true;
        }
        let form = self.layout(app);
        let mut focus = self.focus;
        form.fix_focus(&mut focus);
        let act = {
            let f = focus.and_then(|id| self.field(id));
            form.handle_key(&mut focus, ev, f)
        };
        self.focus = focus;
        self.act(app, act)
    }

    pub fn mouse(&mut self, app: &mut App, ev: &MouseEvent) -> bool {
        if let Some(p) = &mut self.picker {
            let out = p.mouse(ev, crate::ui::screen(app));
            self.picked(out);
            return true;
        }
        let act = self.layout(app).handle_mouse(&mut self.focus, ev);
        self.act(app, act)
    }

    fn act(&mut self, app: &mut App, act: Act<PropId>) -> bool {
        match act {
            Act::Escape | Act::Press(PropId::Cancel) => false,
            Act::Press(PropId::Choose) => {
                let start = match self.dir.text.trim() {
                    "" => app.get(self.id).map(|d| app.settings.dir_for(d.category())).unwrap_or_else(|| app.settings.base_dir()),
                    t => expand_tilde(t),
                };
                self.picker = Some(FolderPicker::new(&start, app.tasks.waker()));
                true
            }
            Act::Submit | Act::Press(PropId::Save) => !self.save(app),
            Act::Step(PropId::Connections, d) => {
                let n = self.connections.map_or(0, |n| n as i32) + d.signum();
                self.connections = (n >= 1).then(|| n.min(32) as u32);
                true
            }
            Act::Step(PropId::Limit, d) => {
                let i = self.limit.and_then(|k| LIMITS.iter().position(|&l| l >= k)).map_or(-1, |i| i as i32) + d.signum();
                self.limit = (i >= 0).then(|| LIMITS[(i as usize).min(LIMITS.len() - 1)]);
                true
            }
            Act::Edited(_) => {
                self.error = None;
                true
            }
            Act::Caret(id, r, col) => {
                if let Some(f) = self.field(id) {
                    f.click(r, col);
                }
                true
            }
            _ => true,
        }
    }

    fn save(&mut self, app: &mut App) -> bool {
        let Some(d) = app.get(self.id) else { return true };
        let url = self.url.text.trim().to_string();
        if crate::links::find(&url).first() != Some(&url) {
            self.error = Some("The address must be a whole link starting with http:// or https://".into());
            self.focus = Some(PropId::Url);
            return false;
        }
        let checksum = match self.checksum.text.trim() {
            "" => None,
            t => match Expected::parse(t) {
                Some(e) => Some(e),
                None => {
                    self.error = Some("The checksum isn't a SHA-256, SHA-1, SHA-512 or MD5 in hex — check it, or leave it empty".into());
                    self.focus = Some(PropId::Checksum);
                    return false;
                }
            },
        };
        let mut edit = Edit {
            url: (url != d.url).then_some(url),
            connections: self.connections,
            speed_limit: self.limit.map(|k| k as u64 * 1024),
            checksum,
            ..Edit::default()
        };
        if self.can_move {
            let name = self.name.text.trim();
            if !name.is_empty() && name != d.name {
                edit.name = Some(crate::engine::http::sanitize_filename(name));
            }
            let dir = match self.dir.text.trim() {
                "" => None,
                t => Some(expand_tilde(t)),
            };
            if dir.as_ref().is_some_and(|p| p.is_relative()) {
                self.error = Some("Give the folder as a full path, like ~/Downloads".into());
                self.focus = Some(PropId::Dir);
                return false;
            }
            if dir != d.dir {
                edit.dir = Some(dir);
            }
        }
        app.edit(self.id, edit);
        true
    }
}

// ---------------------------------------------------------------------------
// Checksums (H)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SumId {
    Compare,
    Copy,
    Close,
}

pub struct Checksum {
    name: String,
    size: u64,
    pub read: Arc<AtomicU64>,
    pub results: Option<Result<Vec<(Algo, String)>, String>>,
    compare: TextField,
    focus: Option<SumId>,
}

pub fn open_checksum(app: &mut App) {
    let Some(d) = app.selected() else { return };
    let Some(path) = d.file().filter(|_| d.status == Status::Done).map(|p| p.to_path_buf()) else {
        app.toast("Checksums are for finished downloads — this one isn't done yet", true);
        return;
    };
    let read = Arc::new(AtomicU64::new(0));
    let r = read.clone();
    let size = d.size.unwrap_or(0);
    let expected = d.checksum.as_ref().map(|c| c.hex.clone()).unwrap_or_default();
    app.modal = Some(Modal::Checksum(Checksum {
        name: d.name.clone(),
        size,
        read,
        results: None,
        compare: TextField::new(expected),
        focus: Some(SumId::Compare),
    }));
    app.tasks.spawn(
        move || checksum::compute(&path, &Algo::ALL, |n| r.store(n, Ordering::Relaxed)).map_err(|e| format::io_reason(&e)),
        |app, result| {
            if let Some(Modal::Checksum(c)) = &mut app.modal {
                c.results = Some(result);
            }
        },
    );
}

impl Checksum {
    fn rects(app: &App) -> (CellRect, CellRect) {
        form::dialog_rects(crate::ui::screen(app), 100, 11)
    }

    /// Still reading the file.
    pub fn busy(&self) -> bool {
        self.results.is_none()
    }

    fn layout(&self, app: &App) -> Form<SumId> {
        let (_, c) = Self::rects(app);
        let mut f = Form::default();
        let lw = 10;
        let want = self.compare.text.trim().to_ascii_lowercase();
        let want = want.rsplit([':', ' ']).next().unwrap_or("").to_string();
        match &self.results {
            None => {
                let pct = (self.read.load(Ordering::Relaxed) * 100).checked_div(self.size).unwrap_or(0);
                f.label(CellRect::new(c.col, c.row, c.cols, 1), format!("Reading {}… {pct} %", self.name), TextKind::Muted);
            }
            Some(Err(e)) => f.label(CellRect::new(c.col, c.row, c.cols, 1), format!("Can't read the file: {e}"), TextKind::Error),
            Some(Ok(sums)) => {
                for (i, (algo, hex)) in sums.iter().enumerate() {
                    let r = c.row + i as u16;
                    f.label(CellRect::new(c.col, r, lw, 1), algo.label(), TextKind::Muted);
                    let kind = if !want.is_empty() && *hex == want { TextKind::Success } else { TextKind::Label };
                    f.label(CellRect::new(c.col + lw, r, c.cols.saturating_sub(lw), 1), hex.clone(), kind);
                }
            }
        }
        f.label(CellRect::new(c.col, c.row + 5, lw, 1), "Compare", TextKind::Label);
        f.ctls.push(Ctl::new(SumId::Compare, CellRect::new(c.col + lw, c.row + 5, c.cols.saturating_sub(lw), 1), Kind::Text(self.compare.clone(), "paste the checksum the website gives".into())));
        if let (Some(Ok(sums)), false) = (&self.results, want.is_empty()) {
            let (text, kind) = match sums.iter().find(|(_, h)| *h == want) {
                Some((a, _)) => (format!("Matches the {} — the file is complete and unchanged", a.label()), TextKind::Success),
                None => ("Doesn't match — the file may be damaged or a different version; download it again with R".into(), TextKind::Error),
            };
            f.label(CellRect::new(c.col + lw, c.row + 6, c.cols.saturating_sub(lw), 1), text, kind);
        }
        let b = form::ButtonRow::new(&["Copy SHA-256", "Close"]).rects(CellRect::new(c.col, c.row + 8, c.cols, 1));
        f.ctls.push(Ctl::new(SumId::Copy, b[0], Kind::Button("Copy SHA-256".into())).disabled(!matches!(self.results, Some(Ok(_)))));
        f.ctls.push(Ctl::new(SumId::Close, b[1], Kind::Button("Close".into())));
        f
    }

    pub fn draw(&self, app: &App, f: &mut Frame, ctx: &DrawCtx) {
        let (outer, _) = Self::rects(app);
        form::dialog_box(f, outer, &format!("Checksums · {}", self.name), ctx);
        self.layout(app).draw(f, self.focus, ctx);
    }

    pub fn key(&mut self, app: &mut App, ev: &KeyEvent) -> bool {
        let form = self.layout(app);
        let mut focus = self.focus;
        form.fix_focus(&mut focus);
        let field = (focus == Some(SumId::Compare)).then_some(&mut self.compare);
        let act = form.handle_key(&mut focus, ev, field);
        self.focus = focus;
        self.act(app, act)
    }

    pub fn mouse(&mut self, app: &mut App, ev: &MouseEvent) -> bool {
        let act = self.layout(app).handle_mouse(&mut self.focus, ev);
        self.act(app, act)
    }

    fn act(&mut self, app: &mut App, act: Act<SumId>) -> bool {
        match act {
            Act::Escape | Act::Press(SumId::Close) => false,
            Act::Press(SumId::Copy) => {
                if let Some(Ok(sums)) = &self.results
                    && let Some((_, h)) = sums.iter().find(|(a, _)| *a == Algo::Sha256)
                {
                    match arboard::Clipboard::new().and_then(|mut c| c.set_text(h.clone())) {
                        Ok(()) => app.toast("Copied the SHA-256", false),
                        Err(e) => app.toast(format!("Can't copy: {e}"), true),
                    }
                }
                true
            }
            Act::Caret(_, r, col) => {
                self.compare.click(r, col);
                true
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Keys (F1, ?)
// ---------------------------------------------------------------------------

const HELP: &[(&str, &str)] = &[
    ("F2  ⌘N", "Add a download (several links, or a [1-20] pattern)"),
    ("⌘V", "Add the links on the clipboard"),
    ("Enter / double-click", "Open a finished file; start or pause the others"),
    ("Space  F5  F6", "Start or pause · start · pause"),
    ("S  P  (⇧F5 ⇧F6)", "Start all · pause all"),
    ("F3  F4", "Open the file · show it in its folder"),
    ("Del  ⇧Del", "Remove from the list · delete the files too"),
    ("R", "Download again from the start"),
    ("E", "Properties: new address, name, folder, limits"),
    ("H", "Checksums of a finished file"),
    ("L  ⌘C", "Copy the link"),
    ("⌥↑ ⌥↓  + −", "Move up or down the queue"),
    ("Tab  1–5", "All · active · paused · completed · failed"),
    ("C", "Filter by file type"),
    ("/  ⌘F", "Search names and addresses (Esc clears)"),
    ("Insert  ⌘A", "Mark · mark all (actions apply to marked ones)"),
    ("F7", "Speed mode: low · medium · high"),
    ("I", "Show or hide the details"),
    ("F9  ⌘,", "Settings"),
    ("⌘+ ⌘− ⌘0", "Font size"),
];

fn help_rect(app: &App) -> CellRect {
    form::dialog_rects(crate::ui::screen(app), 84, HELP.len() as u16 + 2).0
}

pub fn draw_help(app: &App, f: &mut Frame, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let r = help_rect(app);
    form::cover(f, r, ctx);
    let inner = BoxFrame { title: "Keys", subtitle: "Esc closes", active: true }.draw(f, r, ctx);
    let kw = HELP.iter().map(|(k, _)| width(&crate::ui::primary_label(k))).max().unwrap_or(10) + 2;
    for (i, (k, d)) in HELP.iter().enumerate() {
        let row = inner.row + 1 + i as u16;
        if row >= inner.row + inner.rows {
            break;
        }
        put(f, inner.col + 2, row, &crate::ui::primary_label(k), Style::new(c.accent, c.surface).bold(), kw);
        put(f, inner.col + 2 + kw, row, d, Style::new(c.foreground, c.surface), inner.cols.saturating_sub(kw + 3));
    }
}

/// Returns whether the help stays open.
pub fn help_key(ev: &KeyEvent) -> bool {
    !(ev.is(Named::Escape) || ev.is(Named::Enter) || ev.is(Named::F(1)) || ev.typed() == Some("?") || ev.typed() == Some("q"))
}
