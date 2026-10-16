//! Settings (F9, ⌘,): pages on the left, their controls on the right.
//! Changes apply and are saved as they are made (no OK button: the window
//! behind shows them), like Orchard Commander's settings. The dialog keeps
//! only UI state (page, focus, an open list, the text being edited); every
//! value is read from the settings when drawn.
//!
//! Keys: Tab / Shift+Tab and ↑/↓ move between controls, ←/→ change a list
//! or number (Shift: in bigger steps), Enter/Space open a list or toggle;
//! a text field applies its text when the focus leaves it or on Enter.
//! Esc closes (applying the field being edited).

use orchard_grid::{CellRect, Frame, Style};
use orchard_kit::input::{Key, KeyEvent, MouseEvent, MouseKind, Named};
use orchard_kit::widgets::{DrawCtx, TextField, contains, fill, put_elided};
use orchard_theme::schemes::{FONT_SIZE_RANGE, FRAME_STYLES, LINE_SPACING_RANGE, is_builtin};

use crate::app::{App, Modal};
use crate::category::Category;
use crate::format;
use crate::settings::{self, Conflict, ProxyMode, Settings, Traffic, WhenDone};
use crate::ui::add::expand_tilde;
use crate::ui::form::{self, Act, Choice, Ctl, Form, Kind, Popup, PopupOutcome, TextKind};
use orchard_kit::picker::{Outcome, Picker};

pub const PAGES: [&str; 9] = ["Appearance", "Downloads", "Categories", "Connections", "Speed", "Network", "Browser", "Schedule", "When done"];

const WIDTH: u16 = 108;
const SECTIONS_W: u16 = 16;
const PAGE_ROWS: u16 = 15;
const LABEL_W: u16 = 30;
const FIELD_W: u16 = 34;
const SPIN_W: u16 = 16;
const CHOOSE_W: u16 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    // Appearance
    Scheme,
    FollowSystem,
    LightScheme,
    DarkScheme,
    FrameStyle,
    Font,
    FontStyle,
    FontSize,
    LineSpacing,
    // Downloads
    DownloadDir,
    SortByCategory,
    OnConflict,
    StartImmediately,
    RememberFolder,
    ConfirmRemove,
    ForgetAfter,
    // Categories
    CategoryDir(Category),
    // Connections
    MaxActive,
    Connections,
    MinSegment,
    Retries,
    RetryDelay,
    Timeout,
    UserAgent,
    // Speed
    Traffic,
    LowLimit,
    LowMaxActive,
    MediumLimit,
    HighLimit,
    // Network
    ProxyMode,
    Proxy,
    // Browser
    BrowserAsk,
    ClipboardMonitor,
    CatchExtensions,
    // Schedule
    ScheduleEnabled,
    ScheduleStart,
    ScheduleStop,
    Day(usize),
    // When done
    WhenDone,
    PreventSleep,
    NotifyDone,
    VerifyChecksums,
    /// "Choose…" next to a folder field.
    Choose(Folder),
    // Buttons
    ResetPage,
    Close,
}

/// A folder setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Folder {
    Downloads,
    Category(Category),
}

impl Folder {
    fn field(self) -> Id {
        match self {
            Folder::Downloads => Id::DownloadDir,
            Folder::Category(c) => Id::CategoryDir(c),
        }
    }

    fn of(id: Id) -> Option<Folder> {
        match id {
            Id::DownloadDir => Some(Folder::Downloads),
            Id::CategoryDir(c) => Some(Folder::Category(c)),
            _ => None,
        }
    }
}

/// Where the keyboard is: the page list or a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sections,
    Ctl(Id),
}

pub struct SettingsDialog {
    pub page: usize,
    pub focus: Focus,
    pub popup: Option<(Id, Popup)>,
    /// The text field being edited and its text.
    pub editing: Option<(Id, TextField)>,
    /// Color scheme names: built-ins, a separator, the user's.
    schemes: Vec<Choice>,
    /// Why the last text didn't apply.
    pub problem: Option<String>,
    pub(crate) picker: Option<(Folder, Picker)>,
}

/// Open Settings, on `page` if given.
pub fn open(app: &mut App, page: Option<usize>) {
    load_fonts(app);
    let all = orchard_theme::dirs::all_schemes();
    let (builtin, user): (Vec<_>, Vec<_>) = all.into_iter().map(|s| s.name).partition(|n| is_builtin(n));
    let mut schemes: Vec<Choice> = builtin.into_iter().map(Some).collect();
    if !user.is_empty() {
        schemes.push(None);
        schemes.extend(user.into_iter().map(Some));
    }
    app.modal = Some(Modal::Settings(SettingsDialog { page: page.unwrap_or(0), focus: Focus::Sections, popup: None, editing: None, schemes, problem: None, picker: None }));
}

/// Installed monospaced fonts, found on a worker (scanning takes a moment).
fn load_fonts(app: &mut App) {
    if app.fonts.is_some() {
        return;
    }
    app.tasks.spawn(|| orchard_grid::FontLibrary::system().families(true), |app, fonts| app.fonts = Some(fonts));
}

// ---------------------------------------------------------------------------
// What each setting is
// ---------------------------------------------------------------------------

enum Row {
    Heading(&'static str),
    Item(Id),
    /// Monday to Sunday checkboxes on one row.
    Days,
    Gap,
}

fn rows(page: usize, s: &Settings) -> Vec<Row> {
    use Row::*;
    match page {
        0 => {
            let mut v = vec![Heading("Colors")];
            if s.follow_system_appearance {
                v.extend([Item(Id::FollowSystem), Item(Id::LightScheme), Item(Id::DarkScheme)]);
            } else {
                v.extend([Item(Id::Scheme), Item(Id::FollowSystem)]);
            }
            v.extend([Gap, Heading("Window"), Item(Id::FrameStyle), Item(Id::Font), Item(Id::FontStyle), Item(Id::FontSize), Item(Id::LineSpacing)]);
            v
        }
        1 => vec![
            Heading("Saving"),
            Item(Id::DownloadDir),
            Item(Id::SortByCategory),
            Item(Id::OnConflict),
            Item(Id::RememberFolder),
            Gap,
            Heading("The list"),
            Item(Id::StartImmediately),
            Item(Id::ConfirmRemove),
            Item(Id::ForgetAfter),
        ],
        2 => {
            let mut v = vec![Heading("A folder for each file type"), Item(Id::SortByCategory), Gap];
            v.extend(Category::ALL.iter().filter(|&&c| c != Category::Other).map(|&c| Item(Id::CategoryDir(c))));
            v
        }
        3 => vec![
            Heading("At once"),
            Item(Id::MaxActive),
            Item(Id::Connections),
            Item(Id::MinSegment),
            Gap,
            Heading("When a connection fails"),
            Item(Id::Retries),
            Item(Id::RetryDelay),
            Item(Id::Timeout),
            Gap,
            Item(Id::UserAgent),
        ],
        4 => vec![
            Item(Id::Traffic),
            Gap,
            Heading("Low"),
            Item(Id::LowLimit),
            Item(Id::LowMaxActive),
            Heading("Medium"),
            Item(Id::MediumLimit),
            Heading("High"),
            Item(Id::HighLimit),
        ],
        5 => vec![Heading("Proxy"), Item(Id::ProxyMode), Item(Id::Proxy)],
        6 => vec![Heading("Links from the browser extension"), Item(Id::BrowserAsk), Gap, Heading("Links you copy"), Item(Id::ClipboardMonitor), Item(Id::CatchExtensions)],
        7 => vec![Item(Id::ScheduleEnabled), Gap, Item(Id::ScheduleStart), Item(Id::ScheduleStop), Gap, Days],
        _ => vec![
            Heading("After the last download"),
            Item(Id::WhenDone),
            Gap,
            Heading("While downloading"),
            Item(Id::PreventSleep),
            Item(Id::NotifyDone),
            Item(Id::VerifyChecksums),
        ],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Type {
    Button,
    Check,
    Combo,
    Spin,
    Text,
}

fn type_of(id: Id) -> Type {
    use Id::*;
    match id {
        FollowSystem | SortByCategory | StartImmediately | RememberFolder | ConfirmRemove | BrowserAsk | ClipboardMonitor | ScheduleEnabled | Day(_)
        | PreventSleep | NotifyDone | VerifyChecksums => Type::Check,
        Scheme | LightScheme | DarkScheme | FrameStyle | Font | FontStyle | OnConflict | Traffic | ProxyMode | WhenDone => Type::Combo,
        FontSize | LineSpacing | ForgetAfter | MaxActive | Connections | MinSegment | Retries | RetryDelay | Timeout | LowLimit | LowMaxActive
        | MediumLimit | HighLimit => Type::Spin,
        DownloadDir | CategoryDir(_) | UserAgent | Proxy | CatchExtensions | ScheduleStart | ScheduleStop => Type::Text,
        Choose(_) | ResetPage | Close => Type::Button,
    }
}

fn label(id: Id) -> String {
    use Id::*;
    match id {
        Scheme => "Color scheme".into(),
        FollowSystem => "Follow the system's light or dark mode".into(),
        LightScheme => "Light mode scheme".into(),
        DarkScheme => "Dark mode scheme".into(),
        FrameStyle => "Frame style".into(),
        Font => "Font".into(),
        FontStyle => "Font style".into(),
        FontSize => "Font size".into(),
        LineSpacing => "Line spacing".into(),
        DownloadDir => "Download folder".into(),
        SortByCategory => "Sort files into a folder for each type".into(),
        OnConflict => "When the file already exists".into(),
        StartImmediately => "Start new downloads right away".into(),
        RememberFolder => "Suggest the folder used last".into(),
        ConfirmRemove => "Ask before removing downloads".into(),
        ForgetAfter => "Clear finished ones after".into(),
        CategoryDir(c) => c.label().into(),
        MaxActive => "Downloads at once".into(),
        Connections => "Connections per download".into(),
        MinSegment => "Smallest part per connection".into(),
        Retries => "Tries before giving up".into(),
        RetryDelay => "Wait between tries".into(),
        Timeout => "Replace a silent connection after".into(),
        UserAgent => "Browser name sent to servers".into(),
        Traffic => "Speed mode".into(),
        LowLimit | MediumLimit | HighLimit => "Speed limit".into(),
        LowMaxActive => "Downloads at once".into(),
        ProxyMode => "Connect".into(),
        Proxy => "Proxy address".into(),
        BrowserAsk => "Show the Add dialog first".into(),
        ClipboardMonitor => "Offer links copied to the clipboard".into(),
        CatchExtensions => "File types to offer".into(),
        ScheduleEnabled => "Download on a schedule".into(),
        ScheduleStart => "Start at".into(),
        ScheduleStop => "Pause at".into(),
        Day(d) => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][d % 7].into(),
        WhenDone => "Then".into(),
        PreventSleep => "Keep the computer awake".into(),
        NotifyDone => "Show a message when a download finishes".into(),
        VerifyChecksums => "Check files against a checksum given when adding them".into(),
        Choose(_) => "Choose…".into(),
        ResetPage => "Reset page".into(),
        Close => "Close".into(),
    }
}

fn hint(id: Id) -> &'static str {
    use Id::*;
    match id {
        Scheme | LightScheme | DarkScheme => "The colors, shared with Orchard Commander: its built-in schemes and the ones made in its editor. ←/→ preview.",
        FollowSystem => "Switch between a light and a dark scheme with the system's appearance.",
        FrameStyle => "How the boxes around the list and the dialogs are drawn.",
        Font => "Any installed monospaced font.",
        FontStyle => "Regular, a lighter or a bolder weight, as the font has them.",
        FontSize => "Also ⌘+ and ⌘− in the main window; ⌘0 goes back to 14 pt.",
        LineSpacing => "Room between the rows.",
        DownloadDir => "Where files go unless you pick a folder when adding them. Empty: the system's Downloads folder.",
        SortByCategory => "Video, music, documents, archives, programs and images each go to their own folder (set them under Categories).",
        OnConflict => "Keep both (the new file gets a number), replace the old file, skip the download, or ask each time. Reset page forgets the answers remembered by type.",
        StartImmediately => "Off: new downloads wait, paused, until you start them (F5).",
        RememberFolder => "The Add dialog suggests the folder you saved to last; off, files go by type.",
        ConfirmRemove => "Removing from the list keeps finished files; deleting files always asks.",
        ForgetAfter => "Finished downloads leave the list after this long; the files stay.",
        CategoryDir(_) => "Empty: a folder with the type's name in the download folder.",
        MaxActive => "More downloads wait in the queue and start when one finishes.",
        Connections => "Parts of a file fetched at the same time, when the server allows. More can be faster; some servers limit them.",
        MinSegment => "A connection that finishes takes over half of the biggest part left, down to this size.",
        Retries => "A broken connection is tried again this often before the download fails.",
        RetryDelay => "Each try waits longer: this, twice this, three times…",
        Timeout => "A connection that delivers nothing for this long is replaced.",
        UserAgent => "Some servers only serve browsers. Empty: OrchardDownloadManager and its version.",
        Traffic => "F7 or a click on the status line switches modes; each has its own limit.",
        LowLimit | MediumLimit | HighLimit => "For all downloads together. A download can have a lower one of its own (E).",
        LowMaxActive => "In low mode, at most this many downloads run at once.",
        ProxyMode => "System: the proxy in the environment (HTTPS_PROXY, ALL_PROXY…). Manual: the address below.",
        Proxy => "Like http://proxy:8080 or socks5://user:password@host:1080.",
        BrowserAsk => "Off: links and downloads from the browser go straight into the list, started as set under Downloads.",
        ClipboardMonitor => "When you copy a link to a file of a type below, the Add dialog opens with it.",
        CatchExtensions => "File name endings, separated by spaces.",
        ScheduleEnabled => "Queued downloads wait for the start time; running ones pause at the pause time and go on next time.",
        ScheduleStart | ScheduleStop => "Hours and minutes, like 01:30. No pause time: they run until done.",
        Day(_) => "The days the schedule starts downloads.",
        WhenDone => "After a 30-second countdown you can cancel with Esc.",
        PreventSleep => "The computer doesn't go to sleep on its own while downloads run.",
        NotifyDone => "In the app, and as a system notification while its window isn't focused. It never takes the focus.",
        VerifyChecksums => "A file that doesn't match is marked with ! — download it again with R.",
        Choose(_) => "Pick the folder from a list instead of typing it.",
        ResetPage => "Put this page's settings back as they were when the app was new.",
        Close => "Changes are already saved.",
    }
}

// -- values -------------------------------------------------------------

fn check_value(s: &Settings, id: Id) -> bool {
    use Id::*;
    match id {
        FollowSystem => s.follow_system_appearance,
        SortByCategory => s.sort_by_category,
        StartImmediately => s.start_immediately,
        RememberFolder => s.remember_last_folder,
        ConfirmRemove => s.confirm_remove,
        BrowserAsk => s.browser_ask,
        ClipboardMonitor => s.clipboard_monitor,
        ScheduleEnabled => s.schedule_enabled,
        Day(d) => s.schedule_days[d % 7],
        PreventSleep => s.prevent_sleep,
        NotifyDone => s.notify_done,
        VerifyChecksums => s.verify_checksums,
        _ => false,
    }
}

fn toggle(s: &mut Settings, id: Id) {
    use Id::*;
    match id {
        FollowSystem => s.follow_system_appearance ^= true,
        SortByCategory => s.sort_by_category ^= true,
        StartImmediately => s.start_immediately ^= true,
        RememberFolder => s.remember_last_folder ^= true,
        ConfirmRemove => s.confirm_remove ^= true,
        BrowserAsk => s.browser_ask ^= true,
        ClipboardMonitor => s.clipboard_monitor ^= true,
        ScheduleEnabled => s.schedule_enabled ^= true,
        Day(d) => s.schedule_days[d % 7] ^= true,
        PreventSleep => s.prevent_sleep ^= true,
        NotifyDone => s.notify_done ^= true,
        VerifyChecksums => s.verify_checksums ^= true,
        _ => {}
    }
}

/// A combo box's items (labels) and the current one.
fn combo(app: &App, d: &SettingsDialog, id: Id) -> (Vec<Choice>, usize) {
    let s = &app.settings;
    let pick = |items: Vec<Choice>, cur: &str| {
        let i = items.iter().position(|x| x.as_deref() == Some(cur)).unwrap_or(0);
        (items, i)
    };
    let labels = |v: &[&str]| v.iter().map(|x| Some(x.to_string())).collect::<Vec<Choice>>();
    match id {
        Id::Scheme => pick(d.schemes.clone(), &s.color_scheme),
        Id::LightScheme => pick(d.schemes.clone(), &s.light_scheme),
        Id::DarkScheme => pick(d.schemes.clone(), &s.dark_scheme),
        Id::FrameStyle => {
            let items = FRAME_STYLES.iter().map(|(_, l)| Some(l.to_string())).collect();
            let i = FRAME_STYLES.iter().position(|(n, _)| *n == s.frame_style).unwrap_or(0);
            (items, i)
        }
        Id::Font => {
            let mut fonts: Vec<Choice> = vec![Some("Default".into())];
            fonts.extend(app.fonts.clone().unwrap_or_default().into_iter().map(Some));
            if !s.font_family.is_empty() && !fonts.iter().any(|f| f.as_deref() == Some(&s.font_family)) {
                fonts.push(Some(s.font_family.clone()));
            }
            let cur = if s.font_family.is_empty() { "Default" } else { &s.font_family };
            pick(fonts, cur)
        }
        Id::FontStyle => {
            let mut styles: Vec<Choice> = vec![Some("Regular".into())];
            let family = if s.font_family.is_empty() { app.theme.font.family.clone() } else { s.font_family.clone() };
            if app.fonts.is_some() {
                styles.extend(orchard_grid::FontLibrary::system().styles(&family).into_iter().filter(|x| x != "Regular").map(Some));
            }
            let cur = if s.font_style.is_empty() { "Regular" } else { &s.font_style };
            if !styles.iter().any(|x| x.as_deref() == Some(cur)) {
                styles.push(Some(cur.to_string()));
            }
            pick(styles, cur)
        }
        Id::OnConflict => (
            labels(&["Keep both (add a number)", "Replace the old file", "Skip the download", "Ask me"]),
            match s.on_conflict {
                Conflict::Rename => 0,
                Conflict::Overwrite => 1,
                Conflict::Skip => 2,
                Conflict::Ask => 3,
            },
        ),
        Id::Traffic => (labels(&["Low", "Medium", "High"]), Traffic::ALL.iter().position(|&t| t == s.traffic).unwrap_or(2)),
        Id::ProxyMode => (
            labels(&["Use the system's proxy", "Directly, without a proxy", "Through this proxy:"]),
            match s.proxy_mode {
                ProxyMode::System => 0,
                ProxyMode::None => 1,
                ProxyMode::Manual => 2,
            },
        ),
        Id::WhenDone => (
            labels(&["Do nothing", "Quit the app", "Put the computer to sleep"]),
            match s.when_done {
                WhenDone::Nothing => 0,
                WhenDone::Quit => 1,
                WhenDone::Sleep => 2,
            },
        ),
        _ => (Vec::new(), 0),
    }
}

fn choose(app: &mut App, d: &SettingsDialog, id: Id, i: usize) {
    let (items, _) = combo(app, d, id);
    let Some(Some(label)) = items.get(i).cloned() else { return };
    app.update_settings(|s| match id {
        Id::Scheme => s.color_scheme = label,
        Id::LightScheme => s.light_scheme = label,
        Id::DarkScheme => s.dark_scheme = label,
        Id::FrameStyle => s.frame_style = FRAME_STYLES[i.min(FRAME_STYLES.len() - 1)].0.to_string(),
        Id::Font => {
            s.font_family = if i == 0 { String::new() } else { label };
            s.font_style = String::new();
        }
        Id::FontStyle => s.font_style = if label == "Regular" { String::new() } else { label },
        Id::OnConflict => s.on_conflict = [Conflict::Rename, Conflict::Overwrite, Conflict::Skip, Conflict::Ask][i.min(3)],
        Id::Traffic => s.traffic = Traffic::ALL[i.min(2)],
        Id::ProxyMode => s.proxy_mode = [ProxyMode::System, ProxyMode::None, ProxyMode::Manual][i.min(2)],
        Id::WhenDone => s.when_done = [WhenDone::Nothing, WhenDone::Quit, WhenDone::Sleep][i.min(2)],
        _ => {}
    });
}

/// Speed limits (KiB/s) a spin box steps through.
const LIMITS: &[u32] = &[
    16, 32, 64, 128, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 6144, 8192, 12288, 16384, 25600, 51200, 102_400, 204_800, 512_000,
];
const SEGMENTS: &[u32] = &[256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536];
const FORGET: &[u32] = &[0, 1, 3, 7, 14, 30, 60, 90, 180, 365];

fn kib_speed(kib: u32) -> String {
    format::speed(kib as f64 * 1024.0)
}

fn spin_text(s: &Settings, id: Id) -> String {
    match id {
        Id::FontSize => format!("{} pt", s.font_size),
        Id::LineSpacing => format!("{:.2}", s.line_spacing),
        Id::ForgetAfter if s.forget_after_days == 0 => "never".into(),
        Id::ForgetAfter => format::count(s.forget_after_days as usize, "day", "days"),
        Id::MaxActive => s.max_active.to_string(),
        Id::Connections => s.connections.to_string(),
        Id::MinSegment => format::size(s.min_segment_kib as u64 * 1024),
        Id::Retries => s.retries.to_string(),
        Id::RetryDelay => format!("{} s", s.retry_delay_secs),
        Id::Timeout => format!("{} s", s.timeout_secs),
        Id::LowLimit => kib_speed(s.low_limit_kib),
        Id::LowMaxActive => s.low_max_active.to_string(),
        Id::MediumLimit => kib_speed(s.medium_limit_kib),
        Id::HighLimit if s.high_limit_kib == 0 => "no limit".into(),
        Id::HighLimit => kib_speed(s.high_limit_kib),
        _ => String::new(),
    }
}

/// The next value in a list of steps (a value between steps goes to the
/// neighbouring one); big deltas move three steps.
fn step_in(list: &[u32], cur: u32, delta: i32) -> u32 {
    let i = match list.iter().position(|&v| v == cur) {
        Some(i) => i as i32 + delta.signum() * delta.abs().min(3),
        None => {
            let above = list.iter().position(|&v| v > cur).unwrap_or(list.len()) as i32;
            if delta > 0 { above } else { above - 1 }
        }
    };
    list[i.clamp(0, list.len() as i32 - 1) as usize]
}

fn spin(app: &mut App, id: Id, delta: i32) {
    let add = |v: u32, lo: u32, hi: u32| (v as i64 + delta as i64).clamp(lo as i64, hi as i64) as u32;
    app.update_settings(|s| match id {
        Id::FontSize => s.font_size = (s.font_size + delta as i64).clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1),
        Id::LineSpacing => {
            let v = s.line_spacing + delta as f64 * 0.05;
            s.line_spacing = (v.clamp(LINE_SPACING_RANGE.0, LINE_SPACING_RANGE.1) * 100.0).round() / 100.0;
        }
        Id::ForgetAfter => s.forget_after_days = step_in(FORGET, s.forget_after_days, delta),
        Id::MaxActive => s.max_active = add(s.max_active, 1, 20),
        Id::Connections => s.connections = add(s.connections, 1, 32),
        Id::MinSegment => s.min_segment_kib = step_in(SEGMENTS, s.min_segment_kib, delta),
        Id::Retries => s.retries = add(s.retries, 0, 99),
        Id::RetryDelay => s.retry_delay_secs = add(s.retry_delay_secs, 1, 300),
        Id::Timeout => s.timeout_secs = add(s.timeout_secs, 5, 600),
        Id::LowLimit => s.low_limit_kib = step_in(LIMITS, s.low_limit_kib, delta),
        Id::LowMaxActive => s.low_max_active = add(s.low_max_active, 1, 20),
        Id::MediumLimit => s.medium_limit_kib = step_in(LIMITS, s.medium_limit_kib, delta),
        Id::HighLimit => {
            // 0 (no limit) sits after the largest step.
            let top = *LIMITS.last().unwrap();
            s.high_limit_kib = match (s.high_limit_kib, delta > 0) {
                (0, true) => 0,
                (0, false) => top,
                (v, true) if v >= top => 0,
                (v, _) => step_in(LIMITS, v, delta),
            };
        }
        _ => {}
    });
}

fn text_value(s: &Settings, id: Id) -> String {
    match id {
        Id::DownloadDir => s.download_dir.as_deref().map(crate::ui::home_relative).unwrap_or_default(),
        Id::CategoryDir(c) => s.category_dirs.get(&c).map(|p| crate::ui::home_relative(p)).unwrap_or_default(),
        Id::UserAgent => s.user_agent.clone(),
        Id::Proxy => s.proxy.clone(),
        Id::CatchExtensions => s.catch_extensions.clone(),
        Id::ScheduleStart => s.schedule_start.clone(),
        Id::ScheduleStop => s.schedule_stop.clone(),
        _ => String::new(),
    }
}

fn placeholder(app: &App, id: Id) -> String {
    let s = &app.settings;
    match id {
        Id::DownloadDir => dirs::download_dir().map(|d| crate::ui::home_relative(&d)).unwrap_or_default(),
        Id::CategoryDir(c) => crate::ui::home_relative(&s.base_dir().join(c.label())),
        Id::UserAgent => crate::engine::http::default_user_agent(),
        Id::Proxy => "socks5://host:1080 or http://host:8080".into(),
        Id::CatchExtensions => "zip iso mp4 …".into(),
        Id::ScheduleStop => "never".into(),
        _ => String::new(),
    }
}

/// Apply a text field; an error says why it can't.
fn commit_text(app: &mut App, id: Id, text: &str) -> Result<(), String> {
    let t = text.trim().to_string();
    if t == text_value(&app.settings, id).trim() {
        return Ok(());
    }
    let folder = |t: &str| -> Result<Option<std::path::PathBuf>, String> {
        if t.is_empty() {
            return Ok(None);
        }
        let p = expand_tilde(t);
        if p.is_relative() {
            return Err("Give the folder as a full path, like ~/Downloads".into());
        }
        Ok(Some(p))
    };
    match id {
        Id::DownloadDir => {
            let p = folder(&t)?;
            app.update_settings(|s| s.download_dir = p);
        }
        Id::CategoryDir(c) => {
            let p = folder(&t)?;
            app.update_settings(|s| match p {
                Some(p) => {
                    s.category_dirs.insert(c, p);
                }
                None => {
                    s.category_dirs.remove(&c);
                }
            });
        }
        Id::UserAgent => app.update_settings(|s| s.user_agent = t),
        Id::Proxy => {
            if !t.is_empty() {
                let cfg = crate::engine::http::NetConfig { proxy: Some(t.clone()), ..Default::default() };
                crate::engine::http::agent(&cfg).map_err(|e| format!("Can't use this proxy: {e} — write it like http://host:8080"))?;
            }
            app.update_settings(|s| s.proxy = t);
        }
        Id::CatchExtensions => app.update_settings(|s| s.catch_extensions = t),
        Id::ScheduleStart | Id::ScheduleStop => {
            let empty_ok = id == Id::ScheduleStop;
            let value = if t.is_empty() && empty_ok {
                String::new()
            } else {
                let m = settings::parse_hhmm(&t).ok_or("Write the time as hours and minutes, like 01:30")?;
                format!("{:02}:{:02}", m / 60, m % 60)
            };
            app.update_settings(|s| if id == Id::ScheduleStart { s.schedule_start = value } else { s.schedule_stop = value });
        }
        _ => {}
    }
    Ok(())
}

/// This page's settings back to their defaults.
fn reset_page(app: &mut App, page: usize) {
    let def = Settings::default();
    app.update_settings(|s| match page {
        0 => {
            s.color_scheme = def.color_scheme;
            s.follow_system_appearance = def.follow_system_appearance;
            s.light_scheme = def.light_scheme;
            s.dark_scheme = def.dark_scheme;
            s.frame_style = def.frame_style;
            s.font_family = def.font_family;
            s.font_style = def.font_style;
            s.font_size = def.font_size;
            s.line_spacing = def.line_spacing;
        }
        1 => {
            s.download_dir = def.download_dir;
            s.sort_by_category = def.sort_by_category;
            s.on_conflict = def.on_conflict;
            s.conflict_by_type = def.conflict_by_type;
            s.remember_last_folder = def.remember_last_folder;
            s.start_immediately = def.start_immediately;
            s.confirm_remove = def.confirm_remove;
            s.forget_after_days = def.forget_after_days;
        }
        2 => {
            s.sort_by_category = def.sort_by_category;
            s.category_dirs = def.category_dirs;
        }
        3 => {
            s.max_active = def.max_active;
            s.connections = def.connections;
            s.min_segment_kib = def.min_segment_kib;
            s.retries = def.retries;
            s.retry_delay_secs = def.retry_delay_secs;
            s.timeout_secs = def.timeout_secs;
            s.user_agent = def.user_agent;
        }
        4 => {
            s.traffic = def.traffic;
            s.low_limit_kib = def.low_limit_kib;
            s.low_max_active = def.low_max_active;
            s.medium_limit_kib = def.medium_limit_kib;
            s.high_limit_kib = def.high_limit_kib;
        }
        5 => {
            s.proxy_mode = def.proxy_mode;
            s.proxy = def.proxy;
        }
        6 => {
            s.browser_ask = def.browser_ask;
            s.clipboard_monitor = def.clipboard_monitor;
            s.catch_extensions = def.catch_extensions;
        }
        7 => {
            s.schedule_enabled = def.schedule_enabled;
            s.schedule_start = def.schedule_start;
            s.schedule_stop = def.schedule_stop;
            s.schedule_days = def.schedule_days;
        }
        _ => {
            s.when_done = def.when_done;
            s.prevent_sleep = def.prevent_sleep;
            s.notify_done = def.notify_done;
            s.verify_checksums = def.verify_checksums;
        }
    });
}

/// Controls that can't change now (shown muted).
fn disabled(s: &Settings, id: Id) -> bool {
    match id {
        Id::Proxy => s.proxy_mode != ProxyMode::Manual,
        Id::CategoryDir(_) => !s.sort_by_category,
        Id::CatchExtensions => !s.clipboard_monitor,
        Id::ScheduleStart | Id::ScheduleStop | Id::Day(_) => !s.schedule_enabled,
        Id::Choose(f) => disabled(s, f.field()),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Layout and drawing
// ---------------------------------------------------------------------------

struct Rects {
    outer: CellRect,
    sections: CellRect,
    page: CellRect,
    hint: CellRect,
    buttons: CellRect,
}

fn rects(app: &App) -> Rects {
    let (outer, content) = form::dialog_rects(crate::ui::screen(app), WIDTH, PAGE_ROWS + 5);
    let page_rows = content.rows.saturating_sub(5).min(PAGE_ROWS);
    let sections = CellRect::new(content.col, content.row, SECTIONS_W, page_rows);
    let page = CellRect::new(content.col + SECTIONS_W + 2, content.row, content.cols.saturating_sub(SECTIONS_W + 2), page_rows);
    let hint = CellRect::new(content.col, content.row + page_rows + 1, content.cols, 2);
    let buttons = CellRect::new(content.col, content.row + page_rows + 4, content.cols, 1);
    Rects { outer, sections, page, hint, buttons }
}

fn layout(app: &App, d: &SettingsDialog, r: &Rects) -> Form<Id> {
    let s = &app.settings;
    let mut f = Form::default();
    let p = r.page;
    let (x, fx) = (p.col, p.col + LABEL_W);
    let fw = p.cols.saturating_sub(LABEL_W);
    let bottom = p.row + p.rows;
    for (y, row) in (p.row..).zip(rows(d.page, s)) {
        if y >= bottom {
            break;
        }
        match row {
            Row::Heading(h) => f.label(CellRect::new(x, y, p.cols, 1), h, TextKind::Heading),
            Row::Gap => {}
            Row::Days => {
                for day in 0..7 {
                    let id = Id::Day(day);
                    let r = CellRect::new(x + day as u16 * 9, y, 8, 1);
                    f.ctls.push(Ctl::new(id, r, Kind::Check(label(id), check_value(s, id))).disabled(disabled(s, id)));
                }
            }
            Row::Item(id) => {
                let dis = disabled(s, id);
                match type_of(id) {
                    Type::Button => {}
                    Type::Check => f.ctls.push(Ctl::new(id, CellRect::new(x, y, p.cols, 1), Kind::Check(label(id), check_value(s, id))).disabled(dis)),
                    Type::Combo => {
                        f.label(CellRect::new(x, y, LABEL_W - 1, 1), label(id), TextKind::Label);
                        let (items, current) = combo(app, d, id);
                        f.ctls.push(Ctl::new(id, CellRect::new(fx, y, FIELD_W.min(fw), 1), Kind::Combo { items, current }).disabled(dis));
                    }
                    Type::Spin => {
                        f.label(CellRect::new(x, y, LABEL_W - 1, 1), label(id), TextKind::Label);
                        f.ctls.push(Ctl::new(id, CellRect::new(fx, y, SPIN_W.min(fw), 1), Kind::Spin(spin_text(s, id))).disabled(dis));
                    }
                    Type::Text => {
                        f.label(CellRect::new(x, y, LABEL_W - 1, 1), label(id), if dis { TextKind::Muted } else { TextKind::Label });
                        let field = match &d.editing {
                            Some((e, tf)) if *e == id => tf.clone(),
                            _ => TextField::new(text_value(s, id)),
                        };
                        let folder = Folder::of(id);
                        let w = if folder.is_some() { fw.saturating_sub(CHOOSE_W + 1) } else { fw };
                        f.ctls.push(Ctl::new(id, CellRect::new(fx, y, w, 1), Kind::Text(field, placeholder(app, id))).disabled(dis));
                        if let Some(folder) = folder {
                            let b = Id::Choose(folder);
                            f.ctls.push(Ctl::new(b, CellRect::new(fx + w + 1, y, CHOOSE_W, 1), Kind::Button(label(b))).disabled(dis));
                        }
                    }
                }
            }
        }
    }
    let labels = [label(Id::ResetPage), label(Id::Close)];
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let b = form::ButtonRow::new(&refs).rects(r.buttons);
    f.ctls.push(Ctl::new(Id::ResetPage, b[0], Kind::Button(labels[0].clone())));
    f.ctls.push(Ctl::new(Id::Close, b[1], Kind::Button(labels[1].clone())));
    f
}

pub fn draw(app: &App, d: &SettingsDialog, f: &mut Frame, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let r = rects(app);
    form::dialog_box(f, r.outer, "Settings", ctx);
    for (i, title) in PAGES.iter().enumerate() {
        if i as u16 >= r.sections.rows {
            break;
        }
        let row = CellRect::new(r.sections.col, r.sections.row + i as u16, r.sections.cols, 1);
        let style = match (i == d.page, d.focus == Focus::Sections) {
            (true, true) => Style::new(c.cursor_fg, c.cursor_bg),
            (true, false) => Style::new(c.foreground, c.cursor_inactive_bg),
            _ => Style::new(c.foreground, c.surface),
        };
        fill(f, row, style.bg);
        put_elided(f, row.col + 1, row.row, title, style, row.cols.saturating_sub(2));
    }
    let form = layout(app, d, &r);
    let focus = match d.focus {
        Focus::Ctl(id) if d.picker.is_none() => Some(id),
        _ => None,
    };
    form.draw(f, focus, ctx);
    let (text, kind) = match (&d.problem, focus) {
        (Some(p), _) => (p.clone(), c.error),
        (None, Some(id)) => (hint(id).to_string(), c.muted),
        (None, None) => ("↑↓ choose a page · Tab or → its settings · Esc closes · changes are saved as you make them".into(), c.muted),
    };
    for (i, line) in form::wrap(&text, r.hint.cols as usize).iter().take(r.hint.rows as usize).enumerate() {
        put_elided(f, r.hint.col, r.hint.row + i as u16, line, Style::new(kind, c.surface), r.hint.cols);
    }
    if let Some((id, popup)) = &d.popup
        && let Some(Ctl { kind: Kind::Combo { items, .. }, .. }) = form.ctl(*id)
    {
        popup.draw(f, items, crate::ui::screen(app), ctx);
    }
    if let Some((_, p)) = &d.picker {
        p.draw(f, crate::ui::screen(app), ctx);
    }
}

/// The folder picker for a folder setting answered.
fn picked(app: &mut App, d: &mut SettingsDialog, folder: Folder, out: Outcome) {
    match out {
        Outcome::Cancel => d.picker = None,
        Outcome::Folder(p) => {
            d.picker = None;
            d.editing = None;
            match commit_text(app, folder.field(), &p.to_string_lossy()) {
                Ok(()) => d.problem = None,
                Err(e) => d.problem = Some(e),
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

impl SettingsDialog {
    /// Apply the field being edited (if any).
    fn commit(&mut self, app: &mut App) {
        if let Some((id, tf)) = self.editing.take() {
            match commit_text(app, id, &tf.text) {
                Ok(()) => self.problem = None,
                Err(e) => {
                    self.problem = Some(e);
                    self.editing = Some((id, tf));
                }
            }
        }
    }

    fn set_focus(&mut self, app: &mut App, focus: Focus) {
        if self.focus == focus {
            return;
        }
        self.commit(app);
        if self.problem.is_some() {
            return; // stay on the field until it's right (or Esc)
        }
        self.focus = focus;
        if let Focus::Ctl(id) = focus
            && type_of(id) == Type::Text
        {
            self.editing = Some((id, TextField::new(text_value(&app.settings, id))));
        }
    }

    fn set_page(&mut self, app: &mut App, page: usize) {
        self.commit(app);
        if self.problem.is_none() {
            self.page = page.min(PAGES.len() - 1);
        }
    }
}

/// Returns whether the dialog stays open.
pub fn key(app: &mut App, d: &mut SettingsDialog, ev: &KeyEvent) -> bool {
    if let Some((folder, mut p)) = d.picker.take() {
        let out = p.key(ev, crate::ui::screen(app));
        d.picker = Some((folder, p));
        picked(app, d, folder, out);
        return true;
    }
    let r = rects(app);
    let form = layout(app, d, &r);
    if let Some((id, mut popup)) = d.popup.take() {
        if let Some(Ctl { kind: Kind::Combo { items, .. }, .. }) = form.ctl(id) {
            match popup.handle_key(ev, items) {
                PopupOutcome::Consumed => d.popup = Some((id, popup)),
                PopupOutcome::Chosen(i) => choose(app, d, id, i),
                PopupOutcome::Closed => {}
            }
        }
        return true;
    }
    if ev.mods.primary() && matches!(ev.key, Key::Char(',') | Key::Char('w')) {
        d.commit(app);
        return false;
    }
    if d.focus == Focus::Sections {
        match &ev.key {
            Key::Named(Named::Escape | Named::F(9)) => return false,
            Key::Named(Named::Up) => d.set_page(app, d.page.saturating_sub(1)),
            Key::Named(Named::Down) => d.set_page(app, d.page + 1),
            Key::Named(Named::Home) => d.set_page(app, 0),
            Key::Named(Named::End) => d.set_page(app, PAGES.len() - 1),
            Key::Named(Named::Tab | Named::Right | Named::Enter) => {
                let form = layout(app, d, &r);
                let mut first = None;
                form.fix_focus(&mut first);
                if let Some(id) = first {
                    d.set_focus(app, Focus::Ctl(id));
                }
            }
            _ => {
                // A page's first letter jumps to it.
                if let Some(t) = ev.typed() {
                    let t = t.to_lowercase();
                    if let Some(p) = PAGES.iter().position(|p| p.to_lowercase().starts_with(&t)) {
                        d.set_page(app, p);
                    }
                }
            }
        }
        return true;
    }
    let Focus::Ctl(id) = d.focus else { return true };
    // Leaving the controls for the page list: Shift+Tab from the first,
    // ← from anything but a field, list or number.
    let mut focus = Some(id);
    let first = form.ctls.iter().find(|c| !c.disabled).map(|c| c.id);
    if (ev.is(Named::Tab) && ev.mods.shift && first == Some(id)) || (ev.is(Named::Left) && matches!(type_of(id), Type::Check)) {
        d.set_focus(app, Focus::Sections);
        return true;
    }
    if ev.is(Named::Escape) {
        if d.problem.is_some() {
            // Drop the text that doesn't work.
            d.problem = None;
            d.editing = None;
            return true;
        }
        d.commit(app);
        return d.problem.is_some();
    }
    let act = {
        let field = d.editing.as_mut().filter(|(e, _)| *e == id).map(|(_, tf)| tf);
        form.handle_key(&mut focus, ev, field)
    };
    if let Some(f) = focus
        && f != id
    {
        d.set_focus(app, Focus::Ctl(f));
    }
    act_on(app, d, act, &form)
}

pub fn mouse(app: &mut App, d: &mut SettingsDialog, ev: &MouseEvent) -> bool {
    if let Some((folder, mut p)) = d.picker.take() {
        let out = p.mouse(ev, crate::ui::screen(app));
        d.picker = Some((folder, p));
        picked(app, d, folder, out);
        return true;
    }
    let r = rects(app);
    let form = layout(app, d, &r);
    if let Some((id, mut popup)) = d.popup.take() {
        if let Some(Ctl { kind: Kind::Combo { items, .. }, .. }) = form.ctl(id) {
            match popup.handle_mouse(ev, items, crate::ui::screen(app)) {
                PopupOutcome::Consumed => d.popup = Some((id, popup)),
                PopupOutcome::Chosen(i) => choose(app, d, id, i),
                PopupOutcome::Closed => {}
            }
        }
        return true;
    }
    if ev.kind == MouseKind::Down && contains(r.sections, ev.col, ev.row) {
        let p = (ev.row - r.sections.row) as usize;
        if p < PAGES.len() {
            d.set_page(app, p);
            d.set_focus(app, Focus::Sections);
        }
        return true;
    }
    if let MouseKind::Scroll(dy) = ev.kind
        && contains(r.sections, ev.col, ev.row)
    {
        d.set_page(app, if dy > 0.0 { d.page.saturating_sub(1) } else { d.page + 1 });
        return true;
    }
    if ev.kind == MouseKind::Down && !contains(r.outer, ev.col, ev.row) {
        d.commit(app);
        return d.problem.is_some();
    }
    let mut focus = match d.focus {
        Focus::Ctl(id) => Some(id),
        Focus::Sections => None,
    };
    let act = form.handle_mouse(&mut focus, ev);
    if let Some(f) = focus {
        d.set_focus(app, Focus::Ctl(f));
    }
    act_on(app, d, act, &form)
}

/// "Choose…" next to a folder setting: the folder picker, at the folder set
/// or the one used without it.
pub fn choose_folder(app: &App, d: &mut SettingsDialog, folder: Folder) {
    let start = match text_value(&app.settings, folder.field()) {
        t if t.is_empty() => match folder {
            Folder::Downloads => app.settings.base_dir(),
            Folder::Category(c) => app.settings.base_dir().join(c.label()),
        },
        t => expand_tilde(&t),
    };
    d.picker = Some((folder, crate::ui::folder_picker(app, &start)));
}

fn act_on(app: &mut App, d: &mut SettingsDialog, act: Act<Id>, form: &Form<Id>) -> bool {
    match act {
        Act::Escape | Act::Press(Id::Close) => {
            d.commit(app);
            return d.problem.is_some();
        }
        Act::Press(Id::Choose(folder)) => {
            d.commit(app);
            choose_folder(app, d, folder);
        }
        Act::Press(Id::ResetPage) => {
            d.editing = None;
            d.problem = None;
            reset_page(app, d.page);
            app.toast(format!("{} settings reset", PAGES[d.page]), false);
        }
        Act::Toggle(id) => app.update_settings(|s| toggle(s, id)),
        Act::Step(id, delta) => match type_of(id) {
            Type::Combo => {
                let (items, cur) = combo(app, d, id);
                let to = form::step(&items, cur, delta);
                if to != cur {
                    choose(app, d, id, to);
                }
            }
            Type::Spin => spin(app, id, delta),
            _ => {}
        },
        Act::Open(id) => {
            if let Some(Ctl { rect, kind: Kind::Combo { items, current }, .. }) = form.ctl(id) {
                d.popup = Some((id, Popup::open(*rect, items, *current)));
            }
        }
        Act::Submit => {
            d.commit(app);
            if d.problem.is_none() && d.editing.is_none() {
                // Edit again (the focus stays on the field).
                if let Focus::Ctl(id) = d.focus
                    && type_of(id) == Type::Text
                {
                    d.editing = Some((id, TextField::new(text_value(&app.settings, id))));
                }
            }
        }
        Act::Edited(_) => d.problem = None,
        Act::Caret(id, r, col) => {
            if let Some((e, tf)) = &mut d.editing
                && *e == id
            {
                tf.click(r, col);
            }
        }
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_fits_and_every_setting_has_words() {
        let s = Settings { follow_system_appearance: true, ..Settings::default() };
        for (page, name) in PAGES.iter().enumerate() {
            for s in [&s, &Settings::default()] {
                assert!(rows(page, s).len() <= PAGE_ROWS as usize, "page {name} is too long");
            }
            for row in rows(page, &s) {
                if let Row::Item(id) = row {
                    assert!(!label(id).is_empty() && !hint(id).is_empty());
                    assert!(!hint(id).ends_with(' '));
                }
            }
        }
    }

    #[test]
    fn stepping_through_lists() {
        assert_eq!(step_in(LIMITS, 256, 1), 384);
        assert_eq!(step_in(LIMITS, 256, -1), 128);
        assert_eq!(step_in(LIMITS, 300, -1), 256, "between steps: down to the one below");
        assert_eq!(step_in(LIMITS, 16, -1), 16);
        assert_eq!(step_in(FORGET, 0, 1), 1);
    }
}
