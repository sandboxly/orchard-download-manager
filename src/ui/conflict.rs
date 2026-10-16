//! "File already exists": a download's file is already in its folder (or
//! another download in the list is still getting it) and the settings say
//! to ask. Shown in the popup window, one download at a time: keep both
//! (the new file gets a number), replace the old file (not while it is
//! still downloading), or skip the download; optionally the same from now
//! on for the file type.

use std::path::Path;

use orchard_grid::{CellRect, Frame};
use orchard_kit::input::{KeyEvent, MouseEvent};
use orchard_kit::widgets::DrawCtx;

use crate::app::App;
use crate::format;
use crate::settings::{self, Conflict};
use crate::ui::form::{self, Act, Ctl, Form, Kind, TextKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    Remember,
    Keep,
    Replace,
    Skip,
}

pub struct ConflictDialog {
    /// The download asking.
    pub id: u64,
    name: String,
    dir: String,
    /// Another download is still getting the file: no "Replace".
    busy: bool,
    /// The old file and the new one: sizes and the old one's date.
    detail: String,
    pub remember: bool,
    pub focus: Option<Id>,
    /// The popup's grid.
    pub area: CellRect,
}

pub const WIDTH: u16 = 76;
pub const ROWS: u16 = 6;
/// The buttons, in order, and what each one answers.
pub const CHOICES: [(&str, Conflict); 3] = [("Keep both", Conflict::Rename), ("Replace", Conflict::Overwrite), ("Skip", Conflict::Skip)];

/// The buttons offered: no "Replace" while the file is still downloading.
pub fn choices(busy: bool) -> impl Iterator<Item = (&'static str, Conflict)> {
    CHOICES.into_iter().filter(move |(_, c)| !(busy && *c == Conflict::Overwrite))
}

pub fn title(busy: bool) -> &'static str {
    if busy { "Already downloading" } else { "File already exists" }
}

/// "Remember my decision for files of this type (.zip)".
pub fn remember_label(name: &str) -> String {
    match settings::file_type(name).as_str() {
        "" => "Remember my decision for files without an extension".into(),
        t => format!("Remember my decision for files of this type (.{t})"),
    }
}

/// The other download getting the file: "The other download: 1.2 GB of
/// 4.4 GB · paused".
pub fn describe_busy(app: &App, id: u64, path: &Path) -> String {
    let url = app.get(id).map(|d| d.url.as_str()).unwrap_or_default();
    let Some(o) = app.downloading(Some(id), url, path) else {
        return "Another download in the list is getting it".into();
    };
    let got = match o.size {
        Some(n) => format!("{} of {}", format::size(o.received), format::size(n)),
        None => format::size(o.received),
    };
    let state = match o.status {
        crate::download::Status::Running => "downloading",
        crate::download::Status::Queued => "queued",
        crate::download::Status::Paused => "paused",
        crate::download::Status::Failed => "failed",
        crate::download::Status::Done => "done",
    };
    format!("The other download: {got} · {state}")
}

/// "The old file: 12.4 MB from 2026-10-14 15:04 · the new one: 13.0 MB".
pub fn describe(path: &Path, new_size: Option<u64>) -> String {
    let old = match std::fs::metadata(path) {
        Ok(m) => {
            let when = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| format::date(d.as_secs() as i64));
            match when {
                Some(w) => format!("The old file: {} from {w}", format::size(m.len())),
                None => format!("The old file: {}", format::size(m.len())),
            }
        }
        Err(_) => return format!("The new file: {}", new_size.map_or("size unknown".into(), format::size)),
    };
    let new = new_size.map_or("size unknown".into(), format::size);
    format!("{old} · the new one: {new}")
}

impl ConflictDialog {
    pub fn new(app: &App, id: u64, path: &Path, busy: bool) -> ConflictDialog {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = path.parent().map(crate::ui::home_relative).unwrap_or_default();
        let new_size = app.get(id).and_then(|d| d.size);
        ConflictDialog {
            id,
            detail: if busy { describe_busy(app, id, path) } else { describe(path, new_size) },
            name,
            dir,
            busy,
            remember: false,
            focus: Some(Id::Keep),
            area: CellRect::new(0, 0, WIDTH + 4, ROWS + 4),
        }
    }

    fn rects(&self) -> (CellRect, CellRect) {
        form::dialog_rects(self.area, WIDTH, ROWS)
    }

    fn layout(&self) -> Form<Id> {
        let (_, content) = self.rects();
        let mut f = Form::default();
        let row = |i: u16| CellRect::new(content.col, content.row + i, content.cols, 1);
        let what = if self.busy { "is already downloading".to_string() } else { format!("is already in {}", self.dir) };
        f.label(row(0), format!("“{}” {what}", self.name), TextKind::Label);
        f.label(row(1), self.detail.clone(), TextKind::Muted);
        f.ctls.push(Ctl::new(Id::Remember, row(3), Kind::Check(remember_label(&self.name), self.remember)));
        let shown: Vec<(&str, Conflict)> = choices(self.busy).collect();
        let labels: Vec<&str> = shown.iter().map(|(l, _)| *l).collect();
        let b = form::ButtonRow::new(&labels).rects(row(5));
        for ((label, c), r) in shown.into_iter().zip(b) {
            let id = match c {
                Conflict::Overwrite => Id::Replace,
                Conflict::Skip => Id::Skip,
                _ => Id::Keep,
            };
            f.ctls.push(Ctl::new(id, r, Kind::Button(label.into())));
        }
        f
    }

    /// The buttons shown, in order.
    pub fn buttons(&self) -> Vec<Id> {
        self.layout().ctls.iter().map(|c| c.id).filter(|&id| id != Id::Remember).collect()
    }

    pub fn title(&self) -> &'static str {
        title(self.busy)
    }

    pub fn draw(&self, f: &mut Frame, ctx: &DrawCtx) {
        let (outer, _) = self.rects();
        form::dialog_box(f, outer, self.title(), ctx);
        self.layout().draw(f, self.focus, ctx);
    }

    /// Returns whether the dialog stays open.
    pub fn key(&mut self, app: &mut App, ev: &KeyEvent) -> bool {
        let act = self.layout().handle_key(&mut self.focus, ev, None);
        self.act(app, act)
    }

    pub fn mouse(&mut self, app: &mut App, ev: &MouseEvent) -> bool {
        let act = self.layout().handle_mouse(&mut self.focus, ev);
        self.act(app, act)
    }

    fn act(&mut self, app: &mut App, act: Act<Id>) -> bool {
        let choice = match act {
            Act::Toggle(Id::Remember) => {
                self.remember ^= true;
                return true;
            }
            Act::Press(Id::Keep) => Conflict::Rename,
            Act::Press(Id::Replace) => Conflict::Overwrite,
            Act::Press(Id::Skip) => Conflict::Skip,
            // Esc (or closing the window): leave the old file alone, this once.
            Act::Escape => {
                app.answer(Conflict::Skip, false);
                return false;
            }
            _ => return true,
        };
        app.answer(choice, self.remember);
        false
    }
}
