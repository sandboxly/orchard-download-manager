//! The Add dialog (F2, ⌘N, ⌘V, links from the clipboard or another start):
//! one link or many (pasted text, a [1-20] pattern), where to save, an
//! optional name and checksum. A single link is looked up in the
//! background, so the dialog shows its real name and size before adding;
//! if that file already exists (and the settings say to ask), the dialog
//! asks what to do with it. Links from outside the app (the browser, the
//! clipboard, another start) get it in the popup window, the app's own keys
//! in the main window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use orchard_grid::{CellRect, Frame};
use orchard_kit::input::{KeyEvent, MouseEvent};
use orchard_kit::picker::{Outcome, Picker};
use orchard_kit::widgets::{DrawCtx, TextField};

use crate::app::{AddOptions, App, Modal, Popup, PopupDialog};
use crate::category::Category;
use crate::checksum::Expected;
use crate::engine::http::{self, Probe};
use crate::format;
use crate::links;
use crate::settings::{self, Conflict};
use crate::ui::conflict;
use crate::ui::form::{self, Act, Ctl, Form, Kind, TextKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    Links,
    Dir,
    Name,
    Checksum,
    Choose,
    Start,
    /// "Remember my decision…" when the file exists.
    Remember,
    /// "Add download", or "Keep both" when the file exists.
    Add,
    Replace,
    /// "Cancel", or "Skip" when the file exists.
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    None,
    Checking(String),
    Found(String, Probe),
    Failed(String, String),
}

pub struct AddDialog {
    pub links: TextField,
    pub dir: TextField,
    pub name: TextField,
    pub checksum: TextField,
    pub start: bool,
    pub focus: Option<Id>,
    /// Look the link up then (a short pause after typing).
    pub probe_due: Option<Instant>,
    pub lookup: Lookup,
    pub error: Option<String>,
    pub picker: Option<Picker>,
    /// Request headers by link (cookies and the page, from the browser):
    /// for the lookup and the downloads. Never shown.
    pub headers: HashMap<String, http::Headers>,
    pub remember: bool,
    /// In the popup window: its grid (else the main window's).
    pub area: Option<CellRect>,
}

pub const WIDTH: u16 = 92;
pub const ROWS: u16 = 11;
/// The popup's rows while the folder picker is open in it.
pub const PICKER_ROWS: u16 = 32;
const LABEL_W: u16 = 10;
const CHOOSE_W: u16 = 12;

fn new(app: &App, text: &str) -> AddDialog {
    let dir = match &app.settings.last_folder {
        Some(d) if app.settings.remember_last_folder && d.is_dir() => crate::ui::home_relative(d),
        _ => String::new(),
    };
    let mut a = AddDialog {
        links: TextField::new(text),
        dir: TextField::new(dir),
        name: TextField::default(),
        checksum: TextField::default(),
        start: app.settings.start_immediately,
        focus: Some(if text.is_empty() { Id::Links } else { Id::Add }),
        probe_due: None,
        lookup: Lookup::None,
        error: None,
        picker: None,
        headers: HashMap::new(),
        remember: false,
        area: None,
    };
    links_changed(&mut a);
    a
}

/// Open the dialog in the main window with `text` in the links field.
pub fn open(app: &mut App, text: &str) {
    let a = new(app, text);
    app.modal = Some(Modal::Add(Box::new(a)));
}

/// Open the dialog in the popup window with links from outside the app
/// (their headers and, for one, the name the browser chose).
pub fn open_popup(app: &mut App, items: &[crate::ipc::Link]) {
    let text: Vec<&str> = items.iter().map(|l| l.url.as_str()).collect();
    let mut a = new(app, &text.join(" "));
    a.area = Some(CellRect::new(0, 0, WIDTH + 4, ROWS + 4));
    a.headers = items.iter().filter(|l| !l.headers.is_empty()).map(|l| (l.url.clone(), l.headers.clone())).collect();
    if let [one] = items
        && let Some(name) = one.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
    {
        a.name.set_text(name);
    }
    app.open_popup(PopupDialog::Add(Box::new(a)));
}

/// The dialog in the popup window or in the main window.
pub fn dialog(app: &mut App, popup: bool) -> Option<&mut AddDialog> {
    if popup {
        match &mut app.popup {
            Some(Popup { dialog: PopupDialog::Add(a), .. }) => Some(a),
            _ => None,
        }
    } else {
        match &mut app.modal {
            Some(Modal::Add(a)) => Some(a),
            _ => None,
        }
    }
}

/// The links field changed: look a single link up shortly.
pub fn links_changed(a: &mut AddDialog) {
    a.error = None;
    let found = links::find(&a.links.text);
    if found.len() == 1 {
        if !matches!(&a.lookup, Lookup::Checking(u) | Lookup::Found(u, _) | Lookup::Failed(u, _) if *u == found[0]) {
            a.lookup = Lookup::None;
            a.probe_due = Some(Instant::now() + Duration::from_millis(400));
        }
    } else {
        a.lookup = Lookup::None;
        a.probe_due = None;
    }
}

/// Look the single link up (from the frame loop, once typing paused).
pub fn probe(app: &mut App, popup: bool) {
    let Some(a) = dialog(app, popup) else { return };
    let found = links::find(&a.links.text);
    let [url] = found.as_slice() else { return };
    let url = url.clone();
    let headers = a.headers.get(&url).cloned().unwrap_or_default();
    a.lookup = Lookup::Checking(url.clone());
    let agent = match app.agent() {
        Ok(agent) => agent,
        Err(e) => {
            if let Some(a) = dialog(app, popup) {
                a.lookup = Lookup::Failed(url, e);
            }
            return;
        }
    };
    let u = url.clone();
    app.tasks.spawn(
        move || http::probe(&agent, &u, &headers),
        move |app, result| {
            let Some(a) = dialog(app, popup) else { return };
            if a.lookup != Lookup::Checking(url.clone()) {
                return; // the link changed meanwhile
            }
            a.lookup = match result {
                Ok(p) => Lookup::Found(url, p),
                Err(e) => Lookup::Failed(url, e),
            };
        },
    );
}

/// The window's grid the dialog is in.
fn screen(app: &App, a: &AddDialog) -> CellRect {
    a.area.unwrap_or_else(|| crate::ui::screen(app))
}

fn rects(app: &App, a: &AddDialog) -> (CellRect, CellRect) {
    form::dialog_rects(screen(app, a), WIDTH, ROWS)
}

/// The popup's size in cells (taller while the folder picker is open).
pub fn popup_cells(a: &AddDialog) -> (u16, u16) {
    (WIDTH + 4, if a.picker.is_some() { PICKER_ROWS } else { ROWS + 4 })
}

/// A single link's file that is already there.
struct Clash {
    /// Where the file would go.
    path: PathBuf,
    /// Another download in the list is still getting it (from this link
    /// or into this file), rather than a finished file in the folder.
    busy: bool,
}

impl Clash {
    /// What the settings say to do about it.
    fn conflict(&self, app: &App) -> Conflict {
        app.conflict_while(app.settings.conflict_for(&file_name(&self.path)), self.busy)
    }
}

/// For a single link: the file it would save to, if that already exists
/// (once looked up) or another download is getting it.
fn clash(app: &App, a: &AddDialog) -> Option<Clash> {
    let found = links::find(&a.links.text);
    let [url] = found.as_slice() else { return None };
    let name = match a.name.text.trim() {
        "" => auto_name(a),
        n => http::sanitize_filename(n),
    };
    if name.is_empty() {
        return None;
    }
    let dir = match a.dir.text.trim() {
        "" => app.settings.dir_for(Category::of(&name)),
        t => expand_tilde(t),
    };
    let path = dir.join(name);
    if app.downloading(None, url, &path).is_some() {
        return Some(Clash { path, busy: true });
    }
    (matches!(a.lookup, Lookup::Found(..)) && path.is_file()).then_some(Clash { path, busy: false })
}

/// The settings say to ask about it: the dialog asks.
fn asks(app: &App, a: &AddDialog) -> Option<Clash> {
    clash(app, a).filter(|c| c.conflict(app) == Conflict::Ask)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The file name as far as known: the server's, or the link's.
fn auto_name(a: &AddDialog) -> String {
    match &a.lookup {
        Lookup::Found(_, p) => p.name.clone(),
        _ => links::find(&a.links.text).first().map(|l| http::url_filename(l)).unwrap_or_default(),
    }
}

/// The folder a link would go to, as the field's placeholder.
fn auto_dir(app: &App, a: &AddDialog) -> String {
    let name = auto_name(a);
    let several = links::find(&a.links.text).len() > 1;
    if app.settings.sort_by_category && (several || name.is_empty()) {
        return format!("by file type, in {}", crate::ui::home_relative(&app.settings.base_dir()));
    }
    crate::ui::home_relative(&app.settings.dir_for(Category::of(&name)))
}

fn layout(app: &App, a: &AddDialog) -> Form<Id> {
    let (_, content) = rects(app, a);
    let mut f = Form::default();
    let x = content.col;
    let fx = x + LABEL_W;
    let fw = content.cols.saturating_sub(LABEL_W);
    let row = |i: u16| content.row + i;
    let found = links::find(&a.links.text);
    let single = found.len() <= 1;

    f.label(CellRect::new(x, row(0), LABEL_W, 1), "Links", TextKind::Label);
    f.ctls.push(Ctl::new(Id::Links, CellRect::new(fx, row(0), fw, 1), Kind::Text(a.links.clone(), "https://… — or several, or a pattern like file[01-20].jpg".into())));
    let (info, kind) = info_line(app, a, &found);
    f.label(CellRect::new(fx, row(1), fw, 1), info, kind);

    f.label(CellRect::new(x, row(3), LABEL_W, 1), "Save to", TextKind::Label);
    let dir_w = fw.saturating_sub(CHOOSE_W + 1);
    f.ctls.push(Ctl::new(Id::Dir, CellRect::new(fx, row(3), dir_w, 1), Kind::Text(a.dir.clone(), auto_dir(app, a))));
    f.ctls.push(Ctl::new(Id::Choose, CellRect::new(fx + dir_w + 1, row(3), CHOOSE_W, 1), Kind::Button("Choose…".into())));
    f.label(CellRect::new(x, row(4), LABEL_W, 1), "Name", TextKind::Label);
    let name_hint = match (&a.lookup, single) {
        (_, false) => "each file keeps its own name".to_string(),
        (Lookup::Found(_, p), _) => p.name.clone(),
        _ => "from the server".to_string(),
    };
    f.ctls.push(Ctl::new(Id::Name, CellRect::new(fx, row(4), fw, 1), Kind::Text(if single { a.name.clone() } else { TextField::default() }, name_hint)).disabled(!single));
    f.label(CellRect::new(x, row(5), LABEL_W, 1), "Checksum", TextKind::Label);
    f.ctls.push(Ctl::new(
        Id::Checksum,
        CellRect::new(fx, row(5), fw, 1),
        Kind::Text(a.checksum.clone(), "optional: a SHA-256, SHA-1 or MD5 to check the file against".into()),
    ));
    f.ctls.push(Ctl::new(Id::Start, CellRect::new(fx, row(7), fw, 1), Kind::Check("Start right away".into(), a.start)));
    let asking = single.then(|| asks(app, a)).flatten();
    if let Some(c) = &asking {
        f.ctls.push(Ctl::new(Id::Remember, CellRect::new(fx, row(8), fw, 1), Kind::Check(conflict::remember_label(&file_name(&c.path)), a.remember)));
    }
    if let Some(e) = &a.error {
        f.label(CellRect::new(x, row(9), content.cols, 1), e.clone(), TextKind::Error);
    }

    let buttons = CellRect::new(content.col, row(10), content.cols, 1);
    if let Some(c) = &asking {
        let shown: Vec<(&str, Conflict)> = conflict::choices(c.busy).collect();
        let labels: Vec<&str> = shown.iter().map(|(l, _)| *l).collect();
        let b = form::ButtonRow::new(&labels).rects(buttons);
        for ((label, choice), r) in shown.into_iter().zip(b) {
            let id = match choice {
                Conflict::Overwrite => Id::Replace,
                Conflict::Skip => Id::Cancel,
                _ => Id::Add,
            };
            f.ctls.push(Ctl::new(id, r, Kind::Button(label.into())));
        }
        return f;
    }
    let add = if found.len() > 1 { format!("Add {} downloads", found.len()) } else { "Add download".to_string() };
    let labels = [add.as_str(), "Cancel"];
    let b = form::ButtonRow::new(&labels).rects(buttons);
    f.ctls.push(Ctl::new(Id::Add, b[0], Kind::Button(add.clone())));
    f.ctls.push(Ctl::new(Id::Cancel, b[1], Kind::Button("Cancel".into())));
    f
}

/// Under the links: what the lookup found, or how many links there are.
fn info_line(app: &App, a: &AddDialog, found: &[String]) -> (String, TextKind) {
    let known = |l: &str| app.downloads.iter().any(|d| d.url == l);
    match found.len() {
        0 if a.links.text.trim().is_empty() => ("Paste or type the address of a file".into(), TextKind::Muted),
        0 => ("No link here — links start with http:// or https://".into(), TextKind::Error),
        1 => {
            let again = if known(&found[0]) { " · already in the list" } else { "" };
            if let Some(c) = clash(app, a) {
                let name = file_name(&c.path);
                let where_ = if c.busy { "is already downloading".to_string() } else { format!("is already in {}", c.path.parent().map(crate::ui::home_relative).unwrap_or_default()) };
                let what = match c.conflict(app) {
                    Conflict::Ask if c.busy => return (format!("“{name}” {where_}: keep both or skip it"), TextKind::Error),
                    Conflict::Ask => return (format!("“{name}” {where_}: keep both, replace it, or skip it"), TextKind::Error),
                    Conflict::Rename => "the new one gets a number",
                    Conflict::Overwrite => "it will be replaced",
                    Conflict::Skip => "the download will be skipped",
                };
                return (format!("“{name}” {where_}: {what}"), TextKind::Muted);
            }
            match &a.lookup {
                Lookup::Found(_, p) => {
                    let size = p.size.map_or("size unknown".to_string(), format::size);
                    let resume = if p.resumable { "resumable" } else { "can't resume" };
                    (format!("{} · {size} · {resume}{again}", p.name), TextKind::Success)
                }
                Lookup::Failed(_, e) => (format!("Can't reach it: {e}{again}"), TextKind::Error),
                Lookup::Checking(_) | Lookup::None => (format!("Checking the link…{again}"), TextKind::Muted),
            }
        }
        n => {
            let dup = found.iter().filter(|l| known(l)).count();
            let names: Vec<String> = found.iter().take(3).map(|l| http::url_filename(l)).collect();
            let more = if n > 3 { ", …" } else { "" };
            let dup = if dup > 0 { format!(" · {dup} already in the list") } else { String::new() };
            (format!("{n} links: {}{more}{dup}", names.join(", ")), TextKind::Muted)
        }
    }
}

/// The buttons' labels, in order.
pub fn buttons(app: &App, a: &AddDialog) -> Vec<String> {
    layout(app, a)
        .ctls
        .into_iter()
        .filter_map(|c| match c.kind {
            Kind::Button(l) if c.id != Id::Choose => Some(l),
            _ => None,
        })
        .collect()
}

pub fn draw(app: &App, a: &AddDialog, f: &mut Frame, ctx: &DrawCtx) {
    let (outer, _) = rects(app, a);
    form::dialog_box(f, outer, "Add download", ctx);
    let form = layout(app, a);
    let mut focus = a.focus;
    form.fix_focus(&mut focus);
    // Under the picker: no caret showing through it.
    form.draw(f, focus.filter(|_| a.picker.is_none()), ctx);
    if let Some(p) = &a.picker {
        p.draw(f, screen(app, a), ctx);
    }
}

fn field(a: &mut AddDialog, id: Id) -> Option<&mut TextField> {
    match id {
        Id::Links => Some(&mut a.links),
        Id::Dir => Some(&mut a.dir),
        Id::Name => Some(&mut a.name),
        Id::Checksum => Some(&mut a.checksum),
        _ => None,
    }
}

/// Returns whether the dialog stays open.
pub fn key(app: &mut App, a: &mut AddDialog, ev: &KeyEvent) -> bool {
    let area = screen(app, a);
    if let Some(p) = &mut a.picker {
        let out = p.key(ev, area);
        picked(a, out);
        return true;
    }
    let form = layout(app, a);
    let mut focus = a.focus;
    form.fix_focus(&mut focus);
    let act = {
        let f = focus.and_then(|id| field(a, id));
        form.handle_key(&mut focus, ev, f)
    };
    a.focus = focus;
    act_on(app, a, act)
}

pub fn mouse(app: &mut App, a: &mut AddDialog, ev: &MouseEvent) -> bool {
    let area = screen(app, a);
    if let Some(p) = &mut a.picker {
        let out = p.mouse(ev, area);
        picked(a, out);
        return true;
    }
    let form = layout(app, a);
    let act = form.handle_mouse(&mut a.focus, ev);
    act_on(app, a, act)
}

/// The folder picker closed (or not yet).
fn picked(a: &mut AddDialog, out: Outcome) {
    match out {
        Outcome::Cancel => a.picker = None,
        Outcome::Folder(p) => {
            a.dir.set_text(&crate::ui::home_relative(&p));
            a.picker = None;
            a.focus = Some(Id::Add);
        }
        _ => {}
    }
}

/// "Choose…": the folder picker, at the folder typed or the one the file
/// would go to.
pub fn choose(app: &App, a: &mut AddDialog) {
    let start = match a.dir.text.trim() {
        "" => app.settings.dir_for(Category::of(&auto_name(a))),
        t => expand_tilde(t),
    };
    a.picker = Some(crate::ui::folder_picker(app, &start));
}

fn act_on(app: &mut App, a: &mut AddDialog, act: Act<Id>) -> bool {
    match act {
        Act::Press(Id::Choose) => {
            choose(app, a);
            true
        }
        Act::Escape => false,
        Act::Press(Id::Cancel) => {
            // "Skip" when the file exists.
            if a.remember
                && let Some(c) = asks(app, a)
            {
                remember(app, &c.path, Conflict::Skip);
            }
            false
        }
        Act::Submit | Act::Press(Id::Add) => !submit(app, a, Conflict::Rename),
        Act::Press(Id::Replace) => !submit(app, a, Conflict::Overwrite),
        Act::Toggle(Id::Start) => {
            a.start = !a.start;
            true
        }
        Act::Toggle(Id::Remember) => {
            a.remember = !a.remember;
            true
        }
        Act::Edited(Id::Links) => {
            links_changed(a);
            true
        }
        Act::Caret(id, r, col) => {
            if let Some(f) = field(a, id) {
                f.click(r, col);
            }
            true
        }
        _ => true,
    }
}

fn remember(app: &mut App, path: &Path, choice: Conflict) {
    let t = settings::file_type(&file_name(path));
    app.update_settings(|s| {
        s.conflict_by_type.insert(t, choice);
    });
}

/// Add what the dialog says; false (with an error shown) if it can't.
/// `choice`: what to do if the file exists, when the dialog asked.
fn submit(app: &mut App, a: &mut AddDialog, choice: Conflict) -> bool {
    let found = links::find(&a.links.text);
    if found.is_empty() {
        a.error = Some("Add a link first — paste or type an address that starts with http:// or https://".into());
        a.focus = Some(Id::Links);
        return false;
    }
    let checksum = match a.checksum.text.trim() {
        "" => None,
        t => match Expected::parse(t) {
            Some(e) => Some(e),
            None => {
                a.error = Some("The checksum isn't a SHA-256, SHA-1, SHA-512 or MD5 in hex — check it, or leave it empty".into());
                a.focus = Some(Id::Checksum);
                return false;
            }
        },
    };
    let dir = match a.dir.text.trim() {
        "" => None,
        t => Some(expand_tilde(t)),
    };
    if let Some(d) = &dir
        && d.is_relative()
    {
        a.error = Some("Give the folder as a full path, like ~/Downloads".into());
        a.focus = Some(Id::Dir);
        return false;
    }
    let asked = (found.len() == 1).then(|| asks(app, a)).flatten();
    if let Some(c) = &asked
        && a.remember
    {
        remember(app, &c.path, choice);
    }
    // Nothing to replace yet while another download gets the file.
    let conflict = asked.map(|c| if c.busy && choice == Conflict::Overwrite { Conflict::Rename } else { choice });
    let opts = AddOptions { dir, name: Some(a.name.text.clone()), checksum, start: a.start, headers: a.headers.clone(), conflict };
    let n = app.add_links(&found, opts);
    if n > 1 {
        app.toast(format!("Added {}", format::count(n, "download", "downloads")), false);
    }
    true
}

pub fn expand_tilde(text: &str) -> PathBuf {
    match text.strip_prefix("~/").or_else(|| (text == "~").then_some("")) {
        Some(rest) => dirs::home_dir().map_or_else(|| PathBuf::from(text), |h| h.join(rest)),
        None => PathBuf::from(text),
    }
}
