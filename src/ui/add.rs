//! The Add dialog (F2, ⌘N, ⌘V, links from the clipboard or another start):
//! one link or many (pasted text, a [1-20] pattern), where to save, an
//! optional name and checksum. A single link is looked up in the
//! background, so the dialog shows its real name and size before adding.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use oc_grid::{CellRect, Frame};

use crate::app::{AddOptions, App, Modal};
use crate::category::Category;
use crate::checksum::Expected;
use crate::engine::http::{self, Probe};
use crate::format;
use crate::input::{KeyEvent, MouseEvent};
use crate::links;
use crate::ui::form::{self, Act, Ctl, Form, Kind, TextKind};
use crate::ui::picker::{FolderPicker, Outcome};
use crate::ui::widgets::{DrawCtx, TextField};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    Links,
    Dir,
    Name,
    Checksum,
    Choose,
    Start,
    Add,
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
    pub picker: Option<FolderPicker>,
}

const WIDTH: u16 = 92;
const ROWS: u16 = 11;
const LABEL_W: u16 = 10;
const CHOOSE_W: u16 = 12;

/// Open the dialog with `text` in the links field.
pub fn open(app: &mut App, text: &str) {
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
    };
    links_changed(&mut a);
    app.modal = Some(Modal::Add(a));
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
pub fn probe(app: &mut App) {
    let Some(Modal::Add(a)) = &mut app.modal else { return };
    let found = links::find(&a.links.text);
    let [url] = found.as_slice() else { return };
    let url = url.clone();
    a.lookup = Lookup::Checking(url.clone());
    let agent = match app.agent() {
        Ok(agent) => agent,
        Err(e) => {
            if let Some(Modal::Add(a)) = &mut app.modal {
                a.lookup = Lookup::Failed(url, e);
            }
            return;
        }
    };
    let u = url.clone();
    app.tasks.spawn(
        move || http::probe(&agent, &u, &Vec::new()),
        move |app, result| {
            let Some(Modal::Add(a)) = &mut app.modal else { return };
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

fn rects(app: &App) -> (CellRect, CellRect) {
    form::dialog_rects(crate::ui::screen(app), WIDTH, ROWS)
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
    let (_, content) = rects(app);
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
    if let Some(e) = &a.error {
        f.label(CellRect::new(x, row(8), content.cols, 1), e.clone(), TextKind::Error);
    }

    let add = if found.len() > 1 { format!("Add {} downloads", found.len()) } else { "Add download".to_string() };
    let labels = [add.as_str(), "Cancel"];
    let buttons = form::ButtonRow::new(&labels).rects(CellRect::new(content.col, row(10), content.cols, 1));
    f.ctls.push(Ctl::new(Id::Add, buttons[0], Kind::Button(add.clone())));
    f.ctls.push(Ctl::new(Id::Cancel, buttons[1], Kind::Button("Cancel".into())));
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

pub fn draw(app: &App, a: &AddDialog, f: &mut Frame, ctx: &DrawCtx) {
    let (outer, _) = rects(app);
    form::dialog_box(f, outer, "Add download", ctx);
    let form = layout(app, a);
    let mut focus = a.focus;
    form.fix_focus(&mut focus);
    form.draw(f, focus, ctx);
    if let Some(p) = &a.picker {
        p.draw(f, crate::ui::screen(app), ctx);
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
    if let Some(p) = &mut a.picker {
        let out = p.key(ev, crate::ui::screen(app));
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
    if let Some(p) = &mut a.picker {
        let out = p.mouse(ev, crate::ui::screen(app));
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
        Outcome::Keep => {}
        Outcome::Cancel => a.picker = None,
        Outcome::Chosen(p) => {
            a.dir.set_text(&crate::ui::home_relative(&p));
            a.picker = None;
            a.focus = Some(Id::Add);
        }
    }
}

fn act_on(app: &mut App, a: &mut AddDialog, act: Act<Id>) -> bool {
    match act {
        Act::Press(Id::Choose) => {
            let start = match a.dir.text.trim() {
                "" => app.settings.dir_for(Category::of(&auto_name(a))),
                t => expand_tilde(t),
            };
            a.picker = Some(FolderPicker::new(&start, app.tasks.waker()));
            true
        }
        Act::Escape | Act::Press(Id::Cancel) => false,
        Act::Submit | Act::Press(Id::Add) => !submit(app, a),
        Act::Toggle(Id::Start) => {
            a.start = !a.start;
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

/// Add what the dialog says; false (with an error shown) if it can't.
fn submit(app: &mut App, a: &mut AddDialog) -> bool {
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
    let opts = AddOptions { dir, name: Some(a.name.text.clone()), checksum, start: a.start, headers: Vec::new() };
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
