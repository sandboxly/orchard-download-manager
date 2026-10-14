//! Settings (`settings.json`) in the app's config directory. The appearance
//! keys mean what they mean in Orchard Commander's settings; everything
//! else is the download manager's own. Missing keys take their defaults, so
//! older files keep loading.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use oc_config::themes::Appearance;
use serde::{Deserialize, Serialize};

use crate::category::Category;

/// `$ORCHARD_DOWNLOAD_MANAGER_CONFIG_DIR`, or the platform's config folder.
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ORCHARD_DOWNLOAD_MANAGER_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let name = if cfg!(target_os = "linux") { "orchard-download-manager" } else { "Orchard Download Manager" };
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join(name)
}

/// The font size unless changed (Commander's is 13; a little larger here,
/// like the music player).
pub const DEFAULT_FONT_SIZE: i64 = 14;

/// The traffic modes: how much of the connection downloads may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Traffic {
    Low,
    Medium,
    High,
}

impl Traffic {
    pub const ALL: [Traffic; 3] = [Traffic::Low, Traffic::Medium, Traffic::High];

    pub fn label(self) -> &'static str {
        match self {
            Traffic::Low => "low",
            Traffic::Medium => "medium",
            Traffic::High => "high",
        }
    }

    pub fn next(self) -> Traffic {
        match self {
            Traffic::Low => Traffic::Medium,
            Traffic::Medium => Traffic::High,
            Traffic::High => Traffic::Low,
        }
    }
}

/// What happens when the target file already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    /// "name (1).ext".
    Rename,
    Overwrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// The system's (environment / platform) proxy settings.
    System,
    None,
    Manual,
}

/// After the last download finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhenDone {
    Nothing,
    Quit,
    Sleep,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // -- appearance (Commander's keys) ------------------------------------
    pub color_scheme: String,
    /// Use `light_scheme` / `dark_scheme` by the system's light or dark mode.
    pub follow_system_appearance: bool,
    pub light_scheme: String,
    pub dark_scheme: String,
    pub frame_style: String,
    pub font_family: String,
    pub font_style: String,
    pub font_size: i64,
    pub line_spacing: f64,

    // -- downloads --------------------------------------------------------
    /// Where downloads go; `None`: the system's Downloads folder.
    pub download_dir: Option<PathBuf>,
    /// Save each file type into its category's folder.
    pub sort_by_category: bool,
    /// Category folders that differ from "<download dir>/<Category>".
    pub category_dirs: BTreeMap<Category, PathBuf>,
    pub on_conflict: Conflict,
    /// New downloads start right away (else they wait, paused).
    pub start_immediately: bool,
    /// The add dialog suggests the folder used last.
    pub remember_last_folder: bool,
    pub last_folder: Option<PathBuf>,
    /// Ask before removing a download from the list.
    pub confirm_remove: bool,
    /// Remove finished downloads from the list after this many days (0: never).
    pub forget_after_days: u32,

    // -- connections ------------------------------------------------------
    /// Downloads running at once; the rest wait in the queue.
    pub max_active: u32,
    /// Connections per download (segments fetched in parallel).
    pub connections: u32,
    /// Never split a segment smaller than this (KiB).
    pub min_segment_kib: u32,
    /// Tries per connection before a download fails.
    pub retries: u32,
    pub retry_delay_secs: u32,
    /// Give up on a connection that sends nothing for this long.
    pub timeout_secs: u32,
    /// Empty: the app's own.
    pub user_agent: String,

    // -- traffic ----------------------------------------------------------
    pub traffic: Traffic,
    /// KiB/s for the low and medium modes (high is unlimited unless set).
    pub low_limit_kib: u32,
    pub medium_limit_kib: u32,
    /// 0: unlimited.
    pub high_limit_kib: u32,
    /// Downloads at once in low mode (the others keep their own setting).
    pub low_max_active: u32,

    // -- network ----------------------------------------------------------
    pub proxy_mode: ProxyMode,
    /// "http://host:port", "socks5://user:pass@host:port".
    pub proxy: String,

    // -- monitoring -------------------------------------------------------
    /// Links from the browser extension open the Add dialog (else they are
    /// added at once, started as `start_immediately` says).
    pub browser_ask: bool,
    /// Offer links copied to the clipboard.
    pub clipboard_monitor: bool,
    /// File extensions the clipboard monitor reacts to (space separated).
    pub catch_extensions: String,

    // -- scheduler --------------------------------------------------------
    pub schedule_enabled: bool,
    /// "HH:MM": queued downloads start.
    pub schedule_start: String,
    /// "HH:MM": running downloads pause; empty: they run on.
    pub schedule_stop: String,
    /// Monday first.
    pub schedule_days: [bool; 7],

    // -- when done --------------------------------------------------------
    pub when_done: WhenDone,
    /// Keep the computer awake while downloads run.
    pub prevent_sleep: bool,
    /// A message when a download finishes.
    pub notify_done: bool,
    /// Compare files against a checksum given when adding them, right after.
    pub verify_checksums: bool,

    // -- window -----------------------------------------------------------
    pub window_size: Option<(u32, u32)>,
    pub show_details: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let a = Appearance::default();
        Settings {
            color_scheme: a.color_scheme,
            follow_system_appearance: a.follow_system_appearance,
            light_scheme: a.light_scheme,
            dark_scheme: a.dark_scheme,
            frame_style: a.frame_style,
            font_family: a.font_family,
            font_style: a.font_style,
            font_size: DEFAULT_FONT_SIZE,
            line_spacing: a.line_spacing,
            download_dir: None,
            sort_by_category: false,
            category_dirs: BTreeMap::new(),
            on_conflict: Conflict::Rename,
            start_immediately: true,
            remember_last_folder: true,
            last_folder: None,
            confirm_remove: true,
            forget_after_days: 0,
            max_active: 3,
            connections: 8,
            min_segment_kib: 1024,
            retries: 5,
            retry_delay_secs: 5,
            timeout_secs: 30,
            user_agent: String::new(),
            traffic: Traffic::High,
            low_limit_kib: 256,
            medium_limit_kib: 2048,
            high_limit_kib: 0,
            low_max_active: 1,
            proxy_mode: ProxyMode::System,
            proxy: String::new(),
            browser_ask: true,
            clipboard_monitor: true,
            catch_extensions: DEFAULT_CATCH.into(),
            schedule_enabled: false,
            schedule_start: "01:00".into(),
            schedule_stop: "07:00".into(),
            schedule_days: [true; 7],
            when_done: WhenDone::Nothing,
            prevent_sleep: true,
            notify_done: true,
            verify_checksums: true,
            window_size: None,
            show_details: true,
        }
    }
}

/// The clipboard monitor's file types unless changed.
pub const DEFAULT_CATCH: &str = "zip rar 7z gz bz2 xz zst tar tgz iso img dmg pkg exe msi deb rpm appimage apk \
     mp4 mkv avi mov webm m4v wmv flv mp3 flac wav ogg opus m4a aac pdf epub djvu torrent";

impl Settings {
    pub fn appearance(&self) -> Appearance {
        Appearance {
            color_scheme: self.color_scheme.clone(),
            follow_system_appearance: self.follow_system_appearance,
            light_scheme: self.light_scheme.clone(),
            dark_scheme: self.dark_scheme.clone(),
            frame_style: self.frame_style.clone(),
            font_family: self.font_family.clone(),
            font_style: self.font_style.clone(),
            font_size: self.font_size,
            line_spacing: self.line_spacing,
        }
    }

    pub fn load(dir: &Path) -> Settings {
        fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join("settings.json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        fs::rename(tmp, dir.join("settings.json"))
    }

    /// The folder downloads go to unless sorted by category.
    pub fn base_dir(&self) -> PathBuf {
        self.download_dir
            .clone()
            .or_else(dirs::download_dir)
            .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
            .unwrap_or_else(std::env::temp_dir)
    }

    /// The folder a file of `category` goes to.
    pub fn dir_for(&self, category: Category) -> PathBuf {
        if !self.sort_by_category || category == Category::Other {
            return self.base_dir();
        }
        self.category_dirs.get(&category).cloned().unwrap_or_else(|| self.base_dir().join(category.label()))
    }

    /// The speed limit now, in bytes per second (0: none).
    pub fn speed_limit(&self) -> u64 {
        let kib = match self.traffic {
            Traffic::Low => self.low_limit_kib,
            Traffic::Medium => self.medium_limit_kib,
            Traffic::High => self.high_limit_kib,
        };
        kib as u64 * 1024
    }

    /// Downloads at once now.
    pub fn active_limit(&self) -> usize {
        let n = if self.traffic == Traffic::Low { self.low_max_active.min(self.max_active) } else { self.max_active };
        n.max(1) as usize
    }

    pub fn catches(&self, ext: &str) -> bool {
        let ext = ext.to_ascii_lowercase();
        self.catch_extensions.split([' ', ',', ';']).any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
    }
}

/// "HH:MM" as minutes after midnight.
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults_for_missing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings { connections: 4, color_scheme: "nord".into(), traffic: Traffic::Low, ..Settings::default() };
        s.category_dirs.insert(Category::Video, PathBuf::from("/v"));
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        fs::write(dir.path().join("settings.json"), "{\"max_active\": 7}").unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.max_active, 7);
        assert_eq!(s.connections, 8);
    }

    #[test]
    fn limits_follow_the_traffic_mode() {
        let mut s = Settings::default();
        assert_eq!(s.speed_limit(), 0);
        s.traffic = Traffic::Low;
        assert_eq!(s.speed_limit(), 256 * 1024);
        assert_eq!(s.active_limit(), 1);
        s.traffic = Traffic::Medium;
        assert_eq!(s.active_limit(), 3);
    }

    #[test]
    fn category_folders() {
        let mut s = Settings { download_dir: Some("/d".into()), ..Settings::default() };
        assert_eq!(s.dir_for(Category::Video), PathBuf::from("/d"));
        s.sort_by_category = true;
        assert_eq!(s.dir_for(Category::Video), PathBuf::from("/d/Video"));
        assert_eq!(s.dir_for(Category::Other), PathBuf::from("/d"));
        s.category_dirs.insert(Category::Music, "/m".into());
        assert_eq!(s.dir_for(Category::Music), PathBuf::from("/m"));
    }

    #[test]
    fn times_and_extensions() {
        assert_eq!(parse_hhmm("01:30"), Some(90));
        assert_eq!(parse_hhmm("24:00"), None);
        assert!(Settings::default().catches("ZIP"));
        assert!(!Settings::default().catches("html"));
    }
}
