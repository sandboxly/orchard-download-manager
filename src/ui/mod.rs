//! Drawing: a pure function of the app state into an `orchard_grid::Frame`.
//! The layout is recomputed, never stored; mouse hit testing uses it too.

pub mod add;
pub mod dialogs;
pub mod form;
pub mod settings;

use orchard_grid::{CellRect, Flags, Frame, ImagePlacement, Layer, PxRect, Rgba, Style, elide_left, elide_right};
use orchard_kit::input::{KeyEvent, MouseEvent};
use orchard_kit::widgets::{self, BoxFrame, DrawCtx, Scrollbar, contains, draw_key_bar, fill, inset, put, width};
use orchard_theme::mix;

use crate::app::{App, Modal, Tab};
use crate::download::{Download, Status};
use crate::engine::UNKNOWN_END;
use crate::format;
use crate::icons::{self, Icon};
use crate::settings::Traffic;

pub const FKEYS: &[(&str, &str)] = &[
    ("F1", "Help"),
    ("F2", "Add"),
    ("F3", "Open"),
    ("F4", "Folder"),
    ("F5", "Start"),
    ("F6", "Pause"),
    ("F7", "Speed"),
    ("F8", "Remove"),
    ("F9", "Settings"),
    ("F10", "Quit"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    /// The list's box.
    pub main: CellRect,
    pub tabs: CellRect,
    /// The type filter, right on the tab row.
    pub category: Option<CellRect>,
    pub header: CellRect,
    pub list: CellRect,
    /// The search line under the rows, while searching.
    pub search: Option<CellRect>,
    pub details: Option<CellRect>,
    pub status: Option<CellRect>,
    pub footer: CellRect,
}

/// Rows of the details box, frame included.
const DETAILS_H: u16 = 10;

pub fn layout(app: &App) -> Layout {
    let (w, h) = (app.cols.max(30), app.rows.max(8));
    let footer_h = widgets::key_bar_rows(w, FKEYS).min(h / 4).max(1);
    let footer = CellRect::new(0, h - footer_h, w, footer_h);
    let mut avail = h - footer_h;
    let status = (avail > 12).then(|| {
        avail -= 1;
        CellRect::new(0, avail, w, 1)
    });
    let details = (app.settings.show_details && avail >= DETAILS_H + 9).then(|| {
        avail -= DETAILS_H;
        CellRect::new(0, avail, w, DETAILS_H)
    });
    let main = CellRect::new(0, 0, w, avail);
    let inner = inset(main, 1, 1);
    let tabs = CellRect::new(inner.col + 1, inner.row, inner.cols.saturating_sub(2), 1);
    let cat_w = category_label(app).len() as u16 + 2;
    let category = (tabs.cols > 60).then(|| CellRect::new(tabs.col + tabs.cols - cat_w, tabs.row, cat_w, 1));
    let header = CellRect::new(inner.col, inner.row + 2, inner.cols, 1);
    let mut list = CellRect::new(inner.col, inner.row + 3, inner.cols, inner.rows.saturating_sub(3));
    let search = (app.search.is_some() && list.rows > 2).then(|| {
        list.rows -= 1;
        CellRect::new(list.col, list.row + list.rows, list.cols, 1)
    });
    Layout { main, tabs, category, header, list, search, details, status, footer }
}

fn category_label(app: &App) -> String {
    format!("Type: {}", app.category.map_or("all", |c| c.label()))
}

/// The tab strip's tabs and their cells.
fn tabs(app: &App, l: &Layout) -> Vec<(CellRect, Tab, String)> {
    let mut x = l.tabs.col;
    let end = l.category.map_or(l.tabs.col + l.tabs.cols, |c| c.col.saturating_sub(1));
    let mut out = Vec::new();
    for t in Tab::ALL {
        let n = app.tab_count(t);
        let text = format!(" {} {n} ", t.label());
        let w = width(&text);
        if x + w > end {
            break;
        }
        out.push((CellRect::new(x, l.tabs.row, w, 1), t, text));
        x += w + 1;
    }
    out
}

pub fn tab_at(app: &App, l: &Layout, col: u16, row: u16) -> Option<Tab> {
    tabs(app, l).into_iter().find(|(r, ..)| contains(*r, col, row)).map(|(_, t, _)| t)
}

pub fn draw(app: &App, f: &mut Frame) {
    let ctx = DrawCtx { theme: &app.theme, cell_px: app.cell_px };
    let c = &app.theme.colors;
    f.clear(c.background);
    f.cursor = None;
    let l = layout(app);
    draw_list(app, f, &l, &ctx);
    if let Some(r) = l.details {
        draw_details(app, f, r, &ctx);
    }
    if let Some(r) = l.status {
        draw_status(app, f, r, &ctx);
    }
    draw_key_bar(f, l.footer, FKEYS, &ctx);
    if let Some(m) = &app.modal {
        draw_modal(app, m, f, &ctx);
    }
    draw_toast(app, f, &l, &ctx);
}

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

/// Column widths: (size, progress, speed, state); the name gets the rest.
fn columns(w: u16) -> (u16, u16, u16, u16) {
    if w >= 100 {
        (10, 22, 12, 12)
    } else if w >= 72 {
        (10, 16, 11, 10)
    } else {
        (9, 0, 10, 9)
    }
}

fn draw_list(app: &App, f: &mut Frame, l: &Layout, ctx: &DrawCtx) {
    let c = &app.theme.colors;
    let view = app.view();
    let subtitle = if app.marked.is_empty() {
        format::count(view.len(), "download", "downloads")
    } else {
        format!("{} marked of {}", app.marked.len(), view.len())
    };
    BoxFrame { title: "Orchard Download Manager", subtitle: &subtitle, active: true }.draw(f, l.main, ctx);

    // Tabs and the type filter.
    for (r, t, text) in tabs(app, l) {
        let style = if t == app.tab { Style::new(c.cursor_fg, c.cursor_bg).bold() } else { Style::new(c.foreground, mix(c.surface, c.border, 0.35)) };
        put(f, r.col, r.row, &text, style, r.cols);
    }
    if let Some(r) = l.category {
        let style = if app.category.is_some() { Style::new(c.accent, c.surface).bold() } else { Style::new(c.muted, c.surface) };
        put(f, r.col, r.row, &format!("{} ▾", category_label(app)), style, r.cols);
    }

    // Column headers.
    let (size_w, prog_w, speed_w, state_w) = columns(l.header.cols);
    let hs = Style::new(c.muted, c.surface);
    let right_edge = l.header.col + l.header.cols.saturating_sub(1);
    let state_x = right_edge.saturating_sub(state_w);
    let speed_x = state_x.saturating_sub(speed_w + 1);
    let prog_x = speed_x.saturating_sub(prog_w + if prog_w > 0 { 1 } else { 0 });
    let size_x = prog_x.saturating_sub(size_w + 1);
    let name_x = l.header.col + 4;
    put(f, name_x, l.header.row, "Name", hs, size_x.saturating_sub(name_x));
    put_right(f, size_x, l.header.row, size_w, "Size", hs);
    if prog_w > 0 {
        put(f, prog_x, l.header.row, "Progress", hs, prog_w);
    }
    put_right(f, speed_x, l.header.row, speed_w, "Speed", hs);
    put_right(f, state_x, l.header.row, state_w, "Left", hs);

    if let (Some(r), Some(s)) = (l.search, &app.search) {
        draw_search(app, s, f, r, ctx);
    }
    let list = l.list;
    if app.downloads.is_empty() {
        let hints = [
            "No downloads yet",
            "",
            "F2  add a link (or several, or a [1-20] pattern)",
            "⌘V  add the links on the clipboard",
            "Copy a link to a file and it shows up here",
        ];
        let hints: Vec<String> = hints.iter().map(|h| primary_label(h)).collect();
        let top = list.row + list.rows.saturating_sub(hints.len() as u16) / 2;
        for (i, h) in hints.iter().enumerate() {
            let x = list.col + list.cols.saturating_sub(width(h)) / 2;
            let style = if i == 0 { Style::new(c.foreground, c.surface).bold() } else { Style::new(c.muted, c.surface) };
            put(f, x, top + i as u16, h, style, list.cols);
        }
        return;
    }
    if view.is_empty() {
        let msg = if app.search.is_some() { "No downloads match — Esc clears the search" } else { "Nothing here — Tab shows the other lists" };
        let x = list.col + list.cols.saturating_sub(width(msg)) / 2;
        put(f, x, list.row + list.rows / 2, msg, Style::new(c.muted, c.surface), list.cols);
        return;
    }
    for i in 0..list.rows as usize {
        let Some(&idx) = view.get(app.list_top + i) else { break };
        let d = &app.downloads[idx];
        let row = list.row + i as u16;
        let is_cursor = app.cursor == Some(d.id);
        let marked = app.marked.contains(&d.id);
        let bg = if is_cursor { c.cursor_bg } else { c.surface };
        let pick = |normal: Rgba| if is_cursor { c.cursor_fg } else { normal };
        fill(f, CellRect::new(list.col, row, list.cols, 1), bg);
        let (icon, ic) = status_icon(d, c);
        draw_icon(f, CellRect::new(list.col + 1, row, 2, 1), icon, pick(ic), ctx);
        let name_fg = if marked { c.marked } else if d.status == Status::Failed { c.error } else { c.foreground };
        let mut style = Style::new(pick(name_fg), bg);
        if marked {
            style = style.with(Flags::BOLD);
        }
        let room = size_x.saturating_sub(name_x + 1);
        put(f, name_x, row, &elide_right(&d.name, room as usize), style, room);
        let size = match (d.size, d.status) {
            (Some(s), _) => format::size(s),
            (None, Status::Running) if d.received_now() > 0 => format::size(d.received_now()),
            _ => "–".into(),
        };
        put_right(f, size_x, row, size_w, &size, Style::new(pick(c.muted), bg));
        if prog_w > 0 {
            draw_progress(f, CellRect::new(prog_x, row, prog_w, 1), d, is_cursor, ctx);
        }
        let speed = if d.status == Status::Running && d.speed() > 0.0 { format::speed(d.speed()) } else { String::new() };
        put_right(f, speed_x, row, speed_w, &speed, Style::new(pick(c.foreground), bg));
        let (state, sc) = state_text(d, c);
        put_right(f, state_x, row, state_w, &state, Style::new(pick(sc), bg));
    }
    Scrollbar { total: view.len(), visible: list.rows as usize, top: app.list_top }.draw(f, list, ctx);
}

/// "⌘V" on macOS, "Ctrl+V" elsewhere, in hint texts.
pub fn primary_label(text: &str) -> String {
    if cfg!(target_os = "macos") { text.to_string() } else { text.replace('⌘', "Ctrl+") }
}

/// Right-aligned in `w` cells from `x`.
fn put_right(f: &mut Frame, x: u16, row: u16, w: u16, text: &str, style: Style) {
    let t = elide_right(text, w as usize);
    let tw = width(&t);
    put(f, x + w.saturating_sub(tw), row, &t, style, w);
}

fn status_icon(d: &Download, c: &orchard_theme::Colors) -> (Icon, Rgba) {
    match d.status {
        Status::Running => (Icon::Running, c.accent),
        Status::Queued => (Icon::Queued, c.muted),
        Status::Paused => (Icon::Paused, c.warning),
        Status::Done if d.verified == Some(false) => (Icon::Damaged, c.error),
        Status::Done => (Icon::Done, c.success),
        Status::Failed => (Icon::Failed, c.error),
    }
}

/// A status icon, centered in `r`, a little smaller than a row.
pub fn draw_icon(f: &mut Frame, r: CellRect, icon: Icon, color: Rgba, ctx: &DrawCtx) {
    let area = widgets::px(r, ctx);
    let size = (area.h * 0.78).min(area.w).round().max(6.0);
    let dest = PxRect::new((area.x + (area.w - size) / 2.0).round(), (area.y + (area.h - size) / 2.0).round(), size, size);
    f.add_image(ImagePlacement::new(icons::image_id(icon, size as u32, color), dest));
}

/// The last column: time left, or what the download is doing.
fn state_text(d: &Download, c: &orchard_theme::Colors) -> (String, Rgba) {
    match d.status {
        Status::Running if d.run.as_ref().is_some_and(|r| r.stopping) => ("Pausing…".into(), c.muted),
        Status::Running => match d.eta() {
            Some(s) => (format::eta(s), c.foreground),
            None if d.received_now() == 0 => ("Starting…".into(), c.muted),
            None => (String::new(), c.muted),
        },
        Status::Queued => ("Queued".into(), c.muted),
        Status::Paused => ("Paused".into(), c.warning),
        Status::Done if d.verified == Some(false) => ("Damaged".into(), c.error),
        Status::Done => ("Done".into(), c.success),
        Status::Failed => ("Failed".into(), c.error),
    }
}

/// A slim bar and the percentage.
fn draw_progress(f: &mut Frame, r: CellRect, d: &Download, is_cursor: bool, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let bg = if is_cursor { c.cursor_bg } else { c.surface };
    let Some(p) = d.progress() else {
        return;
    };
    let pct = format!("{:>3} %", (p * 100.0).floor() as u32);
    let bar = CellRect::new(r.col, r.row, r.cols.saturating_sub(6), 1);
    put(f, r.col + r.cols - 5, r.row, &pct, Style::new(if is_cursor { c.cursor_fg } else { c.muted }, bg), 5);
    let color = match d.status {
        Status::Done => c.success,
        Status::Failed => c.error,
        Status::Paused => c.warning,
        Status::Queued => c.muted,
        Status::Running => c.accent,
    };
    let track = if is_cursor { mix(c.cursor_bg, c.cursor_fg, 0.25) } else { mix(c.surface, c.border, 0.8) };
    pixel_bar(f, bar, p as f32, color, track, ctx);
}

/// A slim pixel bar: track and the filled part.
fn pixel_bar(f: &mut Frame, r: CellRect, frac: f32, fg: Rgba, track: Rgba, ctx: &DrawCtx) {
    if r.cols == 0 {
        return;
    }
    let area = widgets::px(r, ctx);
    let th = (ctx.cell_px.1 / 4.0).round().max(2.0);
    let y = (area.y + (area.h - th) / 2.0).floor();
    let filled = (area.w * frac.clamp(0.0, 1.0)).round();
    f.add_rect(PxRect::new(area.x, y, area.w, th), track, Layer::BelowText);
    f.add_rect(PxRect::new(area.x, y, filled, th), fg, Layer::BelowText);
}

/// "/ query▏" at the bottom of the list; the field takes keys while editing.
fn draw_search(app: &App, s: &crate::app::Search, f: &mut Frame, r: CellRect, ctx: &DrawCtx) {
    let c = &app.theme.colors;
    let x = put(f, r.col + 1, r.row, "/", Style::new(c.accent, c.surface).bold(), 1) + 1;
    let field = CellRect::new(x, r.row, r.cols.saturating_sub(x - r.col + 1), 1);
    if s.editing {
        s.field.draw(f, field, ctx);
    } else {
        let shown = if s.field.text.is_empty() { "(all)" } else { &s.field.text };
        put(f, field.col, r.row, shown, Style::new(c.foreground, c.surface), field.cols);
        let tip = "/ edit · Esc clear";
        let w = width(tip);
        if field.cols > w + width(shown) + 4 {
            put(f, r.col + r.cols - w - 1, r.row, tip, Style::new(c.muted, c.surface), w);
        }
    }
}

// ---------------------------------------------------------------------------
// Details
// ---------------------------------------------------------------------------

fn draw_details(app: &App, f: &mut Frame, r: CellRect, ctx: &DrawCtx) {
    let c = &app.theme.colors;
    let Some(d) = app.selected() else {
        let inner = BoxFrame { title: "Details", subtitle: "", active: false }.draw(f, r, ctx);
        let msg = "Select a download to see its details";
        put(f, inner.col + inner.cols.saturating_sub(width(msg)) / 2, inner.row + inner.rows / 2, msg, Style::new(c.muted, c.surface), inner.cols);
        return;
    };
    let hint = "I hides";
    let inner = BoxFrame { title: &format!("Details · {}", d.name), subtitle: hint, active: false }.draw(f, r, ctx);
    let graph_w = if inner.cols >= 100 { inner.cols / 3 } else if inner.cols >= 80 { 26 } else { 0 };
    let text_w = inner.cols.saturating_sub(graph_w + if graph_w > 0 { 3 } else { 1 });
    let lx = inner.col + 1;
    let vx = lx + 9;
    let vw = text_w.saturating_sub(10);
    let label = |f: &mut Frame, row: u16, t: &str| {
        put(f, lx, row, t, Style::new(c.muted, c.surface), 9);
    };
    let value = |f: &mut Frame, row: u16, t: &str, fg: Rgba| {
        put(f, vx, row, &elide_right(t, vw as usize), Style::new(fg, c.surface), vw);
    };
    let mut row = inner.row;
    label(f, row, "Address");
    value(f, row, &d.url, c.link);
    row += 1;
    label(f, row, "File");
    match d.file() {
        Some(p) => {
            let shown = home_relative(p);
            put(f, vx, row, &elide_left(&shown, vw as usize), Style::new(c.foreground, c.surface), vw);
        }
        None => {
            let dir = d.dir.clone().unwrap_or_else(|| app.settings.dir_for(d.category()));
            value(f, row, &format!("{} (name from the server)", home_relative(&dir.join(&d.name))), c.muted);
        }
    }
    row += 1;
    label(f, row, "Size");
    let got = d.received_now();
    let size = match (d.size, d.status) {
        (Some(s), Status::Done) => format::size(s),
        (Some(s), _) => format!("{} of {} · {:.1} %", format::size(got), format::size(s), got as f64 * 100.0 / s.max(1) as f64),
        (None, _) if got > 0 => format!("{} so far · size unknown", format::size(got)),
        (None, _) => "not known yet".into(),
    };
    value(f, row, &size, c.foreground);
    row += 1;
    label(f, row, "Status");
    let (status, fg) = status_line(app, d);
    value(f, row, &status, fg);
    row += 1;
    label(f, row, "Speed");
    let mut speed: Vec<String> = Vec::new();
    if d.status == Status::Running {
        speed.push(format::speed(d.speed()));
        if let Some(e) = d.eta() {
            speed.push(format!("{} left", format::eta(e)));
        }
    }
    match d.speed_limit {
        Some(l) => speed.push(format!("limit {}", format::speed(l as f64))),
        None if speed.is_empty() => speed.push("–".into()),
        None => {}
    }
    value(f, row, &speed.join(" · "), c.foreground);
    row += 1;
    label(f, row, "Added");
    let mut when = format::date(d.added);
    if let Some(t) = d.finished {
        when.push_str(&format!(" · finished {}", format::date(t)));
    }
    value(f, row, &when, c.foreground);
    row += 1;
    if row < inner.row + inner.rows {
        label(f, row, "Parts");
        draw_segments(f, CellRect::new(vx, row, vw, 1), d, ctx);
    }
    if graph_w > 0 {
        let g = CellRect::new(inner.col + inner.cols - graph_w - 1, inner.row, graph_w, inner.rows);
        draw_graph(f, g, &d.history, ctx);
    }
}

fn status_line(app: &App, d: &Download) -> (String, Rgba) {
    let c = &app.theme.colors;
    let resumable = match d.resumable {
        Some(true) => " · resumable",
        Some(false) => " · can't resume",
        None => "",
    };
    match d.status {
        Status::Running => {
            let conns = d.run.as_ref().map_or(0, |r| r.shared.connections.load(std::sync::atomic::Ordering::Relaxed));
            let problem = d.run.as_ref().and_then(|r| r.shared.problem.lock().unwrap().clone());
            match problem {
                Some(p) => (format!("Retrying: {p}"), c.warning),
                None => (format!("Downloading · {}{resumable}", format::count(conns, "connection", "connections")), c.success),
            }
        }
        Status::Queued if !app.schedule_allows() => ("Waiting for the schedule".into(), c.muted),
        Status::Queued => ("Waiting for a free slot".into(), c.muted),
        Status::Paused => (format!("Paused{resumable}"), c.warning),
        Status::Done => match d.verified {
            Some(true) => ("Done · checksum matches".into(), c.success),
            Some(false) => ("Done · checksum doesn't match — the file may be damaged".into(), c.error),
            None => ("Done".into(), c.success),
        },
        Status::Failed => (format!("Failed: {}", d.error.as_deref().unwrap_or("unknown error")), c.error),
    }
}

/// The shared folder picker for "Choose…" next to a folder field: places,
/// Orchard Commander's bookmarks (⌘D / Ctrl+D) and the folders, opening at
/// `start` (or the nearest folder above it that exists). Unit tests leave
/// the real bookmarks alone.
pub fn folder_picker(app: &App, start: &std::path::Path) -> orchard_kit::picker::Picker {
    let opts = orchard_kit::picker::Options::folder(start);
    #[cfg(test)]
    let opts = opts.bookmarks(None);
    orchard_kit::picker::Picker::new(opts, app.tasks.waker())
}

/// "~/Downloads/x.zip".
pub fn home_relative(p: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}

/// The file as a bar: received parts filled, connections' positions marked.
fn draw_segments(f: &mut Frame, r: CellRect, d: &Download, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    if r.cols == 0 {
        return;
    }
    let area = widgets::px(r, ctx);
    let th = (ctx.cell_px.1 * 0.55).round().max(4.0);
    let y = (area.y + (area.h - th) / 2.0).floor();
    f.add_rect(PxRect::new(area.x, y, area.w, th), mix(c.surface, c.border, 0.8), Layer::BelowText);
    if d.status == Status::Done {
        f.add_rect(PxRect::new(area.x, y, area.w, th), c.success, Layer::BelowText);
        return;
    }
    let Some(size) = d.size.filter(|&s| s > 0) else { return };
    let x_of = |b: u64| area.x + (b.min(size) as f64 / size as f64 * area.w as f64) as f32;
    let segs = d.segments_now();
    let active: Vec<_> = d.run.as_ref().map(|r| r.shared.active_segments()).unwrap_or_default();
    for s in &segs {
        if s.end == UNKNOWN_END || s.pos <= s.start {
            continue;
        }
        let (a, b) = (x_of(s.start), x_of(s.pos));
        f.add_rect(PxRect::new(a, y, (b - a).max(1.0), th), c.accent, Layer::BelowText);
    }
    for s in &active {
        let x = x_of(s.pos);
        f.add_rect(PxRect::new((x - 1.0).max(area.x), y - 2.0, 2.0, th + 4.0), c.foreground, Layer::BelowText);
    }
}

/// Speed over the last minutes, as bars scaled to the fastest.
fn draw_graph(f: &mut Frame, r: CellRect, history: &std::collections::VecDeque<f32>, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let area = widgets::px(r, ctx);
    f.add_rect(area, mix(c.surface, c.background, 0.5), Layer::BelowText);
    let max = history.iter().copied().fold(0.0f32, f32::max);
    if max <= 0.0 {
        let msg = "Speed shows here";
        put(f, r.col + r.cols.saturating_sub(width(msg)) / 2, r.row + r.rows / 2, msg, Style::new(c.muted, mix(c.surface, c.background, 0.5)), r.cols);
        return;
    }
    let step = 2.0f32;
    let n = (area.w / step) as usize;
    let top = area.y + ctx.cell_px.1;
    let h = area.h - ctx.cell_px.1;
    for (i, v) in history.iter().rev().take(n).enumerate() {
        let bh = (v / max * h).round();
        let x = area.x + area.w - (i + 1) as f32 * step;
        f.add_rect(PxRect::new(x, top + h - bh, step - 0.5, bh), mix(c.accent, c.surface, 0.25), Layer::BelowText);
    }
    let label = format!("max {}", format::speed(max as f64));
    put(f, r.col + 1, r.row, &label, Style::new(c.muted, mix(c.surface, c.background, 0.5)), r.cols.saturating_sub(1));
}

// ---------------------------------------------------------------------------
// The status line
// ---------------------------------------------------------------------------

fn draw_status(app: &App, f: &mut Frame, r: CellRect, _ctx: &DrawCtx) {
    let c = &app.theme.colors;
    fill(f, r, c.background);
    let running = app.running();
    let queued = app.downloads.iter().filter(|d| d.status == Status::Queued).count();
    let mut x = r.col + 1;
    let speed = format!("↓ {}", format::speed(app.total_speed()));
    x = put(f, x, r.row, &speed, Style::new(if running > 0 { c.success } else { c.muted }, c.background).bold(), r.cols) + 2;
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("{running} running"));
    if queued > 0 {
        parts.push(format!("{queued} waiting"));
    }
    let limit = app.settings.speed_limit();
    let mode = app.settings.traffic.label();
    parts.push(if limit == 0 { format!("speed {mode}: no limit") } else { format!("speed {mode}: {}", format::speed(limit as f64)) });
    if app.settings.schedule_enabled {
        let stop = if app.settings.schedule_stop.trim().is_empty() { "…".to_string() } else { app.settings.schedule_stop.clone() };
        parts.push(format!("schedule {}–{stop}", app.settings.schedule_start));
    }
    if app.settings.clipboard_monitor {
        parts.push("watching the clipboard".into());
    }
    let text = parts.join(" · ");
    let style = if app.settings.traffic == Traffic::High { Style::new(c.muted, c.background) } else { Style::new(c.warning, c.background) };
    put(f, x, r.row, &elide_right(&text, (r.col + r.cols).saturating_sub(x + 1) as usize), style, (r.col + r.cols).saturating_sub(x + 1));
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

pub fn screen(app: &App) -> CellRect {
    CellRect::new(0, 0, app.cols, app.rows)
}

fn draw_modal(app: &App, m: &Modal, f: &mut Frame, ctx: &DrawCtx) {
    match m {
        Modal::Add(a) => add::draw(app, a, f, ctx),
        Modal::Settings(s) => settings::draw(app, s, f, ctx),
        Modal::Remove(r) => r.draw(app, f, ctx),
        Modal::Properties(p) => p.draw(app, f, ctx),
        Modal::Checksum(c) => c.draw(app, f, ctx),
        Modal::Help => dialogs::draw_help(app, f, ctx),
    }
}

/// Keys go to the open dialog.
pub fn modal_key(app: &mut App, ev: &KeyEvent) {
    let Some(modal) = app.modal.take() else { return };
    let next = match modal {
        Modal::Add(mut a) => add::key(app, &mut a, ev).then_some(Modal::Add(a)),
        Modal::Settings(mut s) => settings::key(app, &mut s, ev).then_some(Modal::Settings(s)),
        Modal::Remove(mut r) => r.key(app, ev).then_some(Modal::Remove(r)),
        Modal::Properties(mut p) => p.key(app, ev).then_some(Modal::Properties(p)),
        Modal::Checksum(mut c) => c.key(app, ev).then_some(Modal::Checksum(c)),
        Modal::Help => dialogs::help_key(ev).then_some(Modal::Help),
    };
    // Unless the dialog opened another one.
    if app.modal.is_none() {
        app.modal = next;
    }
}

pub fn modal_mouse(app: &mut App, ev: &MouseEvent) {
    let Some(modal) = app.modal.take() else { return };
    let next = match modal {
        Modal::Add(mut a) => add::mouse(app, &mut a, ev).then_some(Modal::Add(a)),
        Modal::Settings(mut s) => settings::mouse(app, &mut s, ev).then_some(Modal::Settings(s)),
        Modal::Remove(mut r) => r.mouse(app, ev).then_some(Modal::Remove(r)),
        Modal::Properties(mut p) => p.mouse(app, ev).then_some(Modal::Properties(p)),
        Modal::Checksum(mut c) => c.mouse(app, ev).then_some(Modal::Checksum(c)),
        Modal::Help => (!matches!(ev.kind, orchard_kit::input::MouseKind::Down)).then_some(Modal::Help),
    };
    if app.modal.is_none() {
        app.modal = next;
    }
}

fn draw_toast(app: &App, f: &mut Frame, l: &Layout, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let row = l.footer.row.saturating_sub(1);
    if let Some(p) = &app.pending {
        let left = p.at.saturating_duration_since(std::time::Instant::now()).as_secs() + 1;
        let what = match p.action {
            crate::settings::WhenDone::Quit => "Quitting",
            crate::settings::WhenDone::Sleep => "Putting the computer to sleep",
            crate::settings::WhenDone::Nothing => return,
        };
        let text = format!(" All downloads done · {what} in {left} s · Esc cancels ");
        let w = width(&text).min(l.footer.cols);
        let x = l.footer.cols.saturating_sub(w) / 2;
        let style = Style::new(c.surface, c.warning).bold();
        fill(f, CellRect::new(x, row, w, 1), style.bg);
        put(f, x, row, &text, style, w);
        return;
    }
    let Some(t) = &app.toast else { return };
    let text = format!(" {} ", elide_right(&t.text, l.footer.cols.saturating_sub(6) as usize));
    let w = width(&text);
    let x = l.footer.cols.saturating_sub(w + 2);
    let style = if t.error { Style::new(c.surface, c.error).bold() } else { Style::new(c.cursor_fg, c.cursor_bg) };
    fill(f, CellRect::new(x, row, w, 1), style.bg);
    put(f, x, row, &text, style, w);
}
