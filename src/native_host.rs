//! The browser extension's helper ("native messaging host"). The browser
//! starts the app with the extension's origin as the argument
//! (`chrome-extension://<id>/`) and talks to it over stdin and stdout: each
//! message is JSON after its length (4 bytes, native byte order). The
//! helper hands links on to the running app (`ipc`), starting it first if
//! it isn't running, and answers:
//!
//! - `{"type": "ping"}` → `{"ok": true, "version": "…"}`
//! - `{"type": "add", "items": [{"url", "referrer", "cookies", "filename"}],
//!   "userAgent": "…"}` → `{"ok": true}` or `{"ok": false, "error": "…"}`
//!
//! The browser finds the helper through a small manifest file (on Windows
//! through the registry), which the app writes on every start
//! ([`register`]).

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::ipc::{Handoff, Link};

/// The helper's name, as the extension asks for it.
pub const HOST_NAME: &str = "io.github.sandboxly.orchard_download_manager";

/// The extensions allowed to talk to the app (the development build's ID
/// comes from the key in its manifest).
pub const EXTENSION_IDS: &[&str] = &["paglhhneednmdpggkmbipgljhgjepaga"];

/// Bigger messages are refused (the browser sends at most a page of links).
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// Started by the browser for its extension.
pub fn is_invocation(args: &[String]) -> bool {
    args.iter().any(|a| a.starts_with("chrome-extension://"))
}

/// The next message, `None` at the end of input.
pub fn read_message(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("a message of {len} bytes")));
    }
    let mut buf = vec![0; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

pub fn write_message(w: &mut impl Write, v: &Value) -> io::Result<()> {
    let text = serde_json::to_vec(v)?;
    w.write_all(&(text.len() as u32).to_ne_bytes())?;
    w.write_all(&text)?;
    w.flush()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Request {
    Ping,
    Add {
        items: Vec<Item>,
        #[serde(default, rename = "userAgent")]
        user_agent: Option<String>,
    },
}

#[derive(Debug, Default, Deserialize)]
pub struct Item {
    pub url: String,
    #[serde(default)]
    pub referrer: Option<String>,
    /// The Cookie header for the link.
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
}

/// The links for the app, with the headers the browser would send.
pub fn handoff(items: Vec<Item>, user_agent: Option<String>) -> Handoff {
    let some = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let user_agent = some(user_agent);
    let items = items
        .into_iter()
        .filter(|i| !i.url.trim().is_empty())
        .map(|i| {
            let mut headers = Vec::new();
            if let Some(c) = some(i.cookies) {
                headers.push(("Cookie".to_string(), c));
            }
            if let Some(r) = some(i.referrer).filter(|r| r.starts_with("http://") || r.starts_with("https://")) {
                headers.push(("Referer".to_string(), r));
            }
            if let Some(ua) = &user_agent {
                headers.push(("User-Agent".to_string(), ua.clone()));
            }
            // Only the file's own name, never a path.
            let name = some(i.filename).map(|n| n.rsplit(['/', '\\']).next().unwrap_or_default().to_string()).filter(|n| !n.is_empty());
            Link { url: i.url.trim().to_string(), headers, name }
        })
        .collect();
    Handoff { links: Vec::new(), items, browser: true }
}

/// The answer to one message; `deliver` hands links to the app.
pub fn answer(message: &[u8], deliver: impl FnOnce(&Handoff) -> Result<(), String>) -> Value {
    match serde_json::from_slice::<Request>(message) {
        Ok(Request::Ping) => json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }),
        Ok(Request::Add { items, user_agent }) => {
            let h = handoff(items, user_agent);
            if h.items.is_empty() {
                return json!({ "ok": false, "error": "No link to add" });
            }
            match deliver(&h) {
                Ok(()) => json!({ "ok": true }),
                Err(e) => json!({ "ok": false, "error": e }),
            }
        }
        Err(e) => {
            log::warn!("browser extension: a message the app doesn't know: {e}");
            json!({ "ok": false, "error": "The app doesn't understand the extension — update both to the same version" })
        }
    }
}

/// Run as the helper until the browser closes the connection.
pub fn run() -> ExitCode {
    let config = crate::settings::config_dir();
    let (mut input, mut output) = (io::stdin().lock(), io::stdout().lock());
    loop {
        let message = match read_message(&mut input) {
            Ok(Some(m)) => m,
            Ok(None) => return ExitCode::SUCCESS,
            Err(e) => {
                log::warn!("browser extension: {e}");
                return ExitCode::FAILURE;
            }
        };
        let reply = answer(&message, |h| deliver(&config, h));
        if write_message(&mut output, &reply).is_err() {
            return ExitCode::FAILURE;
        }
    }
}

/// Hand the links to the running app, starting it if needed.
fn deliver(config: &Path, h: &Handoff) -> Result<(), String> {
    if crate::ipc::send_handoff(config, h) {
        return Ok(());
    }
    start_app()?;
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(250));
        if crate::ipc::send_handoff(config, h) {
            return Ok(());
        }
    }
    Err("Orchard Download Manager didn't start in time — open it, then try again".into())
}

/// Start the app on its own, so it keeps running when the browser ends the
/// helper.
fn start_app() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| {
        log::warn!("browser extension: where is the app? {e}");
        "Can't find Orchard Download Manager — open it by hand, then try again".to_string()
    })?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x8;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // Out of the browser's job, so closing the helper doesn't end the
        // app; where the job doesn't allow that, start it inside.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map(|_| ()).map_err(|e| {
        log::warn!("browser extension: starting {}: {e}", exe.display());
        "Can't start Orchard Download Manager — open it by hand, then try again".to_string()
    })
}

// ---------------------------------------------------------------------------
// Registering the helper with the browsers
// ---------------------------------------------------------------------------

/// The manifest that tells a browser how to start the helper.
pub fn manifest(exe: &Path) -> String {
    let origins: Vec<String> = EXTENSION_IDS.iter().map(|id| format!("chrome-extension://{id}/")).collect();
    let v = json!({
        "name": HOST_NAME,
        "description": "Orchard Download Manager",
        "path": exe,
        "type": "stdio",
        "allowed_origins": origins,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// Write `text` unless the file has it already; true if written.
fn write_if_changed(path: &Path, text: &str) -> io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|t| t == text) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)?;
    Ok(true)
}

/// The browsers' profile folders, under the user's config folder (macOS:
/// Application Support, Linux: ~/.config). Only installed ones get a
/// manifest.
#[cfg(not(windows))]
const BROWSER_DIRS: &[&str] = if cfg!(target_os = "macos") {
    &[
        "Google/Chrome",
        "Google/Chrome Beta",
        "Google/Chrome Dev",
        "Google/Chrome Canary",
        "Chromium",
        "Microsoft Edge",
        "Microsoft Edge Beta",
        "Microsoft Edge Dev",
        "BraveSoftware/Brave-Browser",
        "Vivaldi",
    ]
} else {
    &[
        "google-chrome",
        "google-chrome-beta",
        "google-chrome-unstable",
        "chromium",
        "microsoft-edge",
        "microsoft-edge-beta",
        "microsoft-edge-dev",
        "BraveSoftware/Brave-Browser",
        "vivaldi",
    ]
};

/// Where the manifest goes for the browsers installed under `base`.
#[cfg(not(windows))]
fn manifest_paths(base: &Path) -> Vec<std::path::PathBuf> {
    BROWSER_DIRS
        .iter()
        .map(|d| base.join(d))
        .filter(|d| d.is_dir())
        .map(|d| d.join("NativeMessagingHosts").join(format!("{HOST_NAME}.json")))
        .collect()
}

/// Tell the installed browsers where the helper is (on start; problems go
/// to the log only).
pub fn register() {
    let exe = match std::env::current_exe().and_then(|e| e.canonicalize()) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("browser extension: where is the app? {e}");
            return;
        }
    };
    let text = manifest(&exe);
    #[cfg(not(windows))]
    {
        let Some(base) = dirs::config_dir() else { return };
        for path in manifest_paths(&base) {
            if let Err(e) = write_if_changed(&path, &text) {
                log::warn!("browser extension: writing {}: {e}", path.display());
            }
        }
    }
    #[cfg(windows)]
    {
        let path = crate::settings::config_dir().join("native-messaging-host.json");
        if let Err(e) = write_if_changed(&path, &text) {
            log::warn!("browser extension: writing {}: {e}", path.display());
            return;
        }
        for vendor in WINDOWS_KEYS {
            let key = format!(r"Software\{vendor}\NativeMessagingHosts\{HOST_NAME}");
            if let Err(e) = registry::set_default(&key, &path.to_string_lossy()) {
                log::warn!("browser extension: registry {key}: {e}");
            }
        }
    }
}

/// The browsers' registry folders under HKEY_CURRENT_USER\Software.
#[cfg(windows)]
const WINDOWS_KEYS: &[&str] = &[r"Google\Chrome", "Chromium", r"Microsoft\Edge", r"BraveSoftware\Brave-Browser"];

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegGetValueW,
        RegSetValueExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Set the key's default value to `value`, unless it has it already.
    pub fn set_default(key: &str, value: &str) -> Result<(), String> {
        let key_w = wide(key);
        let value_w = wide(value);
        let mut hkey: HKEY = std::ptr::null_mut();
        // SAFETY: valid NUL-terminated strings and out pointers; the key is
        // closed below.
        unsafe {
            let r = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key_w.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                std::ptr::null(),
                &mut hkey,
                std::ptr::null_mut(),
            );
            if r != ERROR_SUCCESS {
                return Err(format!("error {r}"));
            }
            let mut buf = vec![0u16; value_w.len() + 1];
            let mut size = (buf.len() * 2) as u32;
            let same = RegGetValueW(hkey, std::ptr::null(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
                == ERROR_SUCCESS
                && size as usize == value_w.len() * 2
                && buf[..value_w.len()] == value_w[..];
            let r = if same {
                ERROR_SUCCESS
            } else {
                RegSetValueExW(hkey, std::ptr::null(), 0, REG_SZ, value_w.as_ptr().cast(), (value_w.len() * 2) as u32)
            };
            RegCloseKey(hkey);
            if r != ERROR_SUCCESS {
                return Err(format!("error {r}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(v: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        write_message(&mut out, v).unwrap();
        out
    }

    #[test]
    fn messages_are_length_prefixed_json() {
        let v = json!({ "type": "ping" });
        let bytes = framed(&v);
        assert_eq!(&bytes[..4], &(bytes.len() as u32 - 4).to_ne_bytes());
        let mut r = io::Cursor::new([bytes.clone(), bytes].concat());
        assert_eq!(serde_json::from_slice::<Value>(&read_message(&mut r).unwrap().unwrap()).unwrap(), v);
        assert!(read_message(&mut r).unwrap().is_some());
        assert!(read_message(&mut r).unwrap().is_none(), "the end of input");
    }

    #[test]
    fn huge_or_cut_messages_are_errors() {
        let mut r = io::Cursor::new(u32::MAX.to_ne_bytes().to_vec());
        assert!(read_message(&mut r).is_err());
        let mut cut = framed(&json!({ "type": "ping" }));
        cut.truncate(cut.len() - 2);
        assert!(read_message(&mut io::Cursor::new(cut)).is_err());
    }

    #[test]
    fn started_by_the_browser() {
        assert!(is_invocation(&["chrome-extension://abc/".into()]));
        assert!(is_invocation(&["chrome-extension://abc/".into(), "--parent-window=12".into()]));
        assert!(!is_invocation(&["https://x.org/a.zip".into()]));
    }

    #[test]
    fn ping_says_the_version() {
        let v = answer(br#"{"type":"ping"}"#, |_| panic!("nothing to deliver"));
        assert_eq!(v, json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }));
    }

    #[test]
    fn links_go_with_the_browsers_headers() {
        let msg = br#"{"type":"add","userAgent":"Browser/1.0","items":[
            {"url":"https://x.org/a.zip","referrer":"https://x.org/page","cookies":"s=1; t=2","filename":"/Users/me/Downloads/a.zip"},
            {"url":"https://y.org/b.iso","referrer":"about:blank"},
            {"url":"  "}]}"#;
        let mut got = None;
        let v = answer(msg, |h| {
            got = Some(h.clone());
            Ok(())
        });
        assert_eq!(v, json!({ "ok": true }));
        let h = got.unwrap();
        assert!(h.browser);
        assert_eq!(h.items.len(), 2, "the empty link is dropped");
        let a = &h.items[0];
        assert_eq!(a.name.as_deref(), Some("a.zip"), "only the name, not the browser's path");
        assert_eq!(
            a.headers,
            [("Cookie".into(), "s=1; t=2".into()), ("Referer".into(), "https://x.org/page".into()), ("User-Agent".into(), "Browser/1.0".into())]
        );
        assert_eq!(h.items[1].headers, [("User-Agent".to_string(), "Browser/1.0".to_string())], "no referrer from a blank page");
    }

    #[test]
    fn problems_are_answered() {
        let v = answer(br#"{"type":"add","items":[{"url":"https://x.org/a"}]}"#, |_| Err("Can't start".into()));
        assert_eq!(v, json!({ "ok": false, "error": "Can't start" }));
        assert_eq!(answer(br#"{"type":"add","items":[]}"#, |_| Ok(()))["ok"], false);
        assert_eq!(answer(b"{\"type\":\"dance\"}", |_| Ok(()))["ok"], false);
        assert_eq!(answer(b"not json", |_| Ok(()))["ok"], false);
    }

    #[test]
    fn the_manifest_names_the_app_and_the_extension() {
        let v: Value = serde_json::from_str(&manifest(Path::new("/Apps/odm"))).unwrap();
        assert_eq!(v["name"], HOST_NAME);
        assert_eq!(v["path"], "/Apps/odm");
        assert_eq!(v["type"], "stdio");
        assert_eq!(v["allowed_origins"][0], format!("chrome-extension://{}/", EXTENSION_IDS[0]));
    }

    #[test]
    fn manifests_are_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("NativeMessagingHosts").join("x.json");
        assert!(write_if_changed(&path, "a").unwrap());
        assert!(!write_if_changed(&path, "a").unwrap());
        assert!(write_if_changed(&path, "b").unwrap());
    }

    #[cfg(not(windows))]
    #[test]
    fn only_installed_browsers_get_a_manifest() {
        let dir = tempfile::tempdir().unwrap();
        assert!(manifest_paths(dir.path()).is_empty());
        std::fs::create_dir_all(dir.path().join(BROWSER_DIRS[0])).unwrap();
        assert_eq!(manifest_paths(dir.path()), [dir.path().join(BROWSER_DIRS[0]).join("NativeMessagingHosts").join(format!("{HOST_NAME}.json"))]);
    }
}
