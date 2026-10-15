//! A folder picker, over the dialog that opened it ("Choose…" next to a
//! folder field). Folders are listed on a thread of their own (a slow or
//! network disk never stalls the window).
//!
//! On Windows, ".." from a drive root leads to "Drives": one entry per
//! drive letter (from the drive bitmask, without touching the drives: an
//! empty card reader can hang). Drives can't be chosen itself; going up
//! from C:\ puts the cursor on "C:".

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orchard_grid::{CellRect, Frame, Style, elide_left};
use orchard_kit::input::{Key, KeyEvent, MouseEvent, MouseKind, Named};
use orchard_kit::tasks::Waker;
use orchard_kit::widgets::{DrawCtx, Scrollbar, contains, fill, inset, put_elided};

use crate::ui::form::{self, ButtonRow};

/// What the picker shows: a folder, or (Windows) the drives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Dir(PathBuf),
    Drives,
}

impl Place {
    /// The place ".." leads to.
    pub fn parent(&self) -> Option<Place> {
        match self {
            Place::Drives => None,
            Place::Dir(d) => match d.parent() {
                Some(p) => Some(Place::Dir(p.to_path_buf())),
                None if cfg!(windows) => Some(Place::Drives),
                None => None,
            },
        }
    }

    pub fn title(&self) -> String {
        match self {
            Place::Drives => "Drives".into(),
            Place::Dir(d) => crate::ui::home_relative(d),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub place: Place,
}

/// The listing a thread fills in.
#[derive(Default)]
struct Listing {
    place: Option<Place>,
    entries: Option<Result<Vec<Entry>, String>>,
}

pub struct FolderPicker {
    pub place: Place,
    listing: Arc<Mutex<Listing>>,
    pub cursor: usize,
    pub top: usize,
    /// Put the cursor on this entry when the listing arrives.
    select: Option<Place>,
    search: String,
    search_at: Option<Instant>,
    buttons: ButtonRow,
    waker: Waker,
}

/// What the picker asks of the dialog that opened it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Keep,
    Cancel,
    Chosen(PathBuf),
}

impl FolderPicker {
    /// Open at `start` (or the nearest folder above it that exists).
    pub fn new(start: &Path, waker: Waker) -> FolderPicker {
        let mut dir = start.to_path_buf();
        while !dir.is_dir() {
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => {
                    dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                    break;
                }
            }
        }
        let mut p = FolderPicker {
            place: Place::Dir(dir.clone()),
            listing: Arc::default(),
            cursor: 0,
            top: 0,
            select: None,
            search: String::new(),
            search_at: None,
            buttons: ButtonRow::new(&["Use this folder", "Cancel"]),
            waker,
        };
        p.go(Place::Dir(dir), None);
        p
    }

    fn go(&mut self, place: Place, select: Option<Place>) {
        self.place = place.clone();
        self.select = select;
        self.cursor = 0;
        self.top = 0;
        *self.listing.lock().unwrap() = Listing { place: Some(place.clone()), entries: None };
        let listing = self.listing.clone();
        let waker = self.waker.clone();
        let _ = std::thread::Builder::new().name("list-folder".into()).spawn(move || {
            let entries = list(&place);
            let mut l = listing.lock().unwrap();
            if l.place.as_ref() == Some(&place) {
                l.entries = Some(entries);
            }
            drop(l);
            waker();
        });
    }

    /// The entries, if listed (".." is row 0 when there is a parent).
    fn entries(&self) -> Option<Vec<Entry>> {
        match &self.listing.lock().unwrap().entries {
            Some(Ok(e)) => Some(e.clone()),
            Some(Err(_)) => Some(Vec::new()),
            None => None,
        }
    }

    /// The listing arrived.
    pub fn listed(&self) -> bool {
        self.listing.lock().unwrap().entries.is_some()
    }

    fn error(&self) -> Option<String> {
        match &self.listing.lock().unwrap().entries {
            Some(Err(e)) => Some(e.clone()),
            _ => None,
        }
    }

    fn has_parent(&self) -> bool {
        self.place.parent().is_some()
    }

    fn rows(&self) -> usize {
        self.entries().map_or(0, |e| e.len()) + usize::from(self.has_parent())
    }

    /// The entry at a row (`None` for "..").
    fn entry(&self, row: usize) -> Option<Entry> {
        let i = row.checked_sub(usize::from(self.has_parent()))?;
        self.entries()?.get(i).cloned()
    }

    /// The cursor, on the entry asked for once that is listed.
    fn effective_cursor(&self) -> usize {
        let Some(want) = &self.select else { return self.cursor };
        let Some(entries) = self.entries() else { return self.cursor };
        entries.iter().position(|e| &e.place == want).map_or(self.cursor, |i| i + usize::from(self.has_parent()))
    }

    /// Move the cursor onto the entry asked for, once listed.
    fn settle_cursor(&mut self) {
        if self.select.is_some() && self.entries().is_some() {
            self.cursor = self.effective_cursor();
            self.select = None;
        }
    }

    /// The first row shown for `h` rows, keeping `cursor` in view.
    fn top_for(&self, cursor: usize, h: usize) -> usize {
        let h = h.max(1);
        let top = if cursor < self.top {
            cursor
        } else if cursor >= self.top + h {
            cursor + 1 - h
        } else {
            self.top
        };
        top.min(self.rows().saturating_sub(h))
    }

    fn up(&mut self) {
        if let Some(p) = self.place.parent() {
            let from = self.place.clone();
            self.go(p, Some(from));
        }
    }

    fn open(&mut self, row: usize) {
        if row == 0 && self.has_parent() {
            self.up();
        } else if let Some(e) = self.entry(row) {
            self.go(e.place, None);
        }
    }

    fn chosen(&self) -> Outcome {
        match &self.place {
            Place::Dir(d) => Outcome::Chosen(d.clone()),
            Place::Drives => Outcome::Keep,
        }
    }

    pub fn rects(screen: CellRect) -> (CellRect, CellRect, CellRect, CellRect) {
        let h = screen.rows.saturating_sub(6).clamp(8, 30);
        let (outer, content) = form::dialog_rects(screen, 76, h);
        let list = CellRect::new(content.col, content.row, content.cols, content.rows.saturating_sub(3));
        let hint = CellRect::new(content.col, content.row + content.rows.saturating_sub(2), content.cols, 1);
        let buttons = CellRect::new(content.col, content.row + content.rows.saturating_sub(1), content.cols, 1);
        (outer, list, hint, buttons)
    }

    fn scroll_into_view(&mut self, h: usize) {
        self.top = self.top_for(self.cursor, h);
    }

    pub fn key(&mut self, ev: &KeyEvent, screen: CellRect) -> Outcome {
        self.settle_cursor();
        let (_, list, ..) = Self::rects(screen);
        let n = self.rows();
        let page = list.rows.max(2) as usize - 1;
        let mv = |c: usize, d: isize| (c as isize + d).clamp(0, n.saturating_sub(1) as isize) as usize;
        if ev.mods.primary() && ev.is(Named::Enter) || ev.is(Named::F(2)) {
            return self.chosen();
        }
        match &ev.key {
            Key::Named(Named::Escape) => return Outcome::Cancel,
            Key::Named(Named::Up) => self.cursor = mv(self.cursor, -1),
            Key::Named(Named::Down) => self.cursor = mv(self.cursor, 1),
            Key::Named(Named::PageUp) => self.cursor = mv(self.cursor, -(page as isize)),
            Key::Named(Named::PageDown) => self.cursor = mv(self.cursor, page as isize),
            Key::Named(Named::Home) => self.cursor = 0,
            Key::Named(Named::End) => self.cursor = n.saturating_sub(1),
            Key::Named(Named::Enter | Named::Right) => self.open(self.cursor),
            Key::Named(Named::Backspace | Named::Left) => self.up(),
            Key::Named(Named::Tab) => {
                self.buttons.focus = (self.buttons.focus + 1) % 2;
            }
            _ => match ev.typed() {
                Some("~") => {
                    if let Some(h) = dirs::home_dir() {
                        self.go(Place::Dir(h), None);
                    }
                }
                Some(t) => self.find(t),
                None => {}
            },
        }
        self.scroll_into_view(list.rows as usize);
        Outcome::Keep
    }

    /// Typing jumps to the first folder starting with what was typed.
    fn find(&mut self, text: &str) {
        let now = Instant::now();
        if self.search_at.is_none_or(|t| now.duration_since(t) > Duration::from_millis(900)) {
            self.search.clear();
        }
        self.search_at = Some(now);
        self.search.push_str(&text.to_lowercase());
        if let Some(r) = (0..self.rows()).find(|&r| self.entry(r).is_some_and(|e| e.name.to_lowercase().starts_with(&self.search))) {
            self.cursor = r;
        }
    }

    pub fn mouse(&mut self, ev: &MouseEvent, screen: CellRect) -> Outcome {
        self.settle_cursor();
        let (outer, list, _, buttons) = Self::rects(screen);
        match ev.kind {
            MouseKind::Scroll(dy) if contains(list, ev.col, ev.row) => {
                let max = self.rows().saturating_sub(list.rows as usize);
                self.top = (self.top as f32 - dy * 3.0).round().clamp(0.0, max as f32) as usize;
            }
            MouseKind::Down if contains(list, ev.col, ev.row) => {
                let row = self.top + (ev.row - list.row) as usize;
                if row < self.rows() {
                    self.cursor = row;
                    if ev.double {
                        self.open(row);
                    }
                }
            }
            MouseKind::Down => match self.buttons.hit(buttons, ev.col, ev.row) {
                Some(0) => return self.chosen(),
                Some(_) => return Outcome::Cancel,
                None if !contains(outer, ev.col, ev.row) => return Outcome::Cancel,
                None => {}
            },
            _ => {}
        }
        Outcome::Keep
    }

    pub fn draw(&self, f: &mut Frame, screen: CellRect, ctx: &DrawCtx) {
        let c = &ctx.theme.colors;
        let (outer, list, hint, buttons) = Self::rects(screen);
        let title = format!("Choose a folder · {}", self.place.title());
        let title = elide_left(&title, outer.cols.saturating_sub(8) as usize);
        form::dialog_box(f, outer, &title, ctx);
        let cursor = self.effective_cursor();
        let top = self.top_for(cursor, list.rows as usize);
        let rows = self.rows();
        match self.entries() {
            None => {
                put_elided(f, list.col, list.row, "Listing…", Style::new(c.muted, c.surface), list.cols);
            }
            Some(_) => {
                for i in 0..list.rows as usize {
                    let r = top + i;
                    if r >= rows {
                        break;
                    }
                    let row = CellRect::new(list.col, list.row + i as u16, list.cols, 1);
                    let style = if r == cursor { Style::new(c.cursor_fg, c.cursor_bg) } else { Style::new(c.dir, c.surface) };
                    fill(f, row, style.bg);
                    let name = match self.entry(r) {
                        None => "..".to_string(),
                        Some(e) if e.place == Place::Drives => e.name,
                        Some(e) => format!("{}/", e.name),
                    };
                    put_elided(f, row.col + 1, row.row, &name, style, row.cols.saturating_sub(2));
                }
                if let Some(e) = self.error() {
                    put_elided(f, list.col + 1, list.row + 1, &format!("Can't open this folder: {e}"), Style::new(c.error, c.surface), list.cols.saturating_sub(2));
                } else if rows == usize::from(self.has_parent()) {
                    put_elided(f, list.col + 1, list.row + 1, "No folders in here — use this one or go up", Style::new(c.muted, c.surface), list.cols.saturating_sub(2));
                }
                Scrollbar { total: rows, visible: list.rows as usize, top }.draw(f, inset(list, 0, 0), ctx);
            }
        }
        let tip = crate::ui::primary_label("Enter open · ⌫ up · ~ home · F2 or ⌘Enter use this folder · Esc cancel");
        put_elided(f, hint.col, hint.row, &tip, Style::new(c.muted, c.surface), hint.cols);
        self.buttons.draw(f, buttons, ctx, self.place != Place::Drives);
    }
}

/// The subfolders of a place, by name (blocking).
fn list(place: &Place) -> Result<Vec<Entry>, String> {
    match place {
        Place::Drives => Ok(drives().into_iter().map(|d| Entry { place: Place::Dir(PathBuf::from(format!("{d}\\"))), name: d }).collect()),
        Place::Dir(dir) => {
            let rd = std::fs::read_dir(dir).map_err(|e| crate::format::io_reason(&e))?;
            let mut v: Vec<Entry> = rd
                .flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .filter(|e| e.path().is_dir())
                .map(|e| Entry { name: e.file_name().to_string_lossy().into_owned(), place: Place::Dir(e.path()) })
                .collect();
            v.sort_by_key(|e| e.name.to_lowercase());
            Ok(v)
        }
    }
}

/// "C:", "D:", … from the drive bitmask (Windows; nothing elsewhere).
pub fn drives() -> Vec<String> {
    #[cfg(windows)]
    {
        // SAFETY: no arguments, returns a bitmask.
        let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        drive_letters(mask)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Bit 0 is A:, bit 1 B:, ….
pub fn drive_letters(mask: u32) -> Vec<String> {
    (0..26u8).filter(|i| mask & (1 << i) != 0).map(|i| format!("{}:", (b'A' + i) as char)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(p: &FolderPicker) {
        let until = Instant::now() + Duration::from_secs(5);
        while p.entries().is_none() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn lists_folders_goes_in_and_back_up() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("b")).unwrap();
        std::fs::create_dir(d.path().join("A")).unwrap();
        std::fs::write(d.path().join("file.txt"), b"").unwrap();
        let screen = CellRect::new(0, 0, 100, 40);
        let mut p = FolderPicker::new(d.path(), Arc::new(|| {}));
        wait(&p);
        let names: Vec<String> = p.entries().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["A", "b"], "folders only, by name");
        p.key(&KeyEvent::char('b'), screen);
        assert_eq!(p.cursor, 2);
        p.key(&KeyEvent::named(Named::Enter), screen);
        assert_eq!(p.place, Place::Dir(d.path().join("b")));
        wait(&p);
        p.key(&KeyEvent::named(Named::Backspace), screen);
        wait(&p);
        p.settle_cursor();
        assert_eq!(p.cursor, 2, "back on the folder we came from");
        assert_eq!(p.key(&KeyEvent::named(Named::F(2)), screen), Outcome::Chosen(d.path().to_path_buf()));
        assert_eq!(p.key(&KeyEvent::named(Named::Escape), screen), Outcome::Cancel);
    }

    #[test]
    fn a_missing_start_opens_the_nearest_existing_folder() {
        let d = tempfile::tempdir().unwrap();
        let p = FolderPicker::new(&d.path().join("not/there"), Arc::new(|| {}));
        assert_eq!(p.place, Place::Dir(d.path().to_path_buf()));
    }

    #[test]
    fn drive_letters_from_the_mask() {
        assert_eq!(drive_letters(0b1101), ["A:", "C:", "D:"]);
        assert!(Place::Drives.parent().is_none());
        assert_eq!(Place::Drives.title(), "Drives");
        if cfg!(windows) {
            assert_eq!(Place::Dir(PathBuf::from("C:\\")).parent(), Some(Place::Drives));
        } else {
            assert_eq!(Place::Dir(PathBuf::from("/")).parent(), None);
        }
    }
}
