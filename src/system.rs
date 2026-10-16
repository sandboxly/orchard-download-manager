//! The operating system: opening files, showing them in the file manager,
//! keeping the computer awake while downloading, putting it to sleep,
//! notifications.

use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Open a file with its default app.
pub fn open(path: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(path);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("explorer");
        c.arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    spawn(&mut cmd)
}

/// Show a file selected in the file manager (its folder where that can't select).
pub fn reveal(path: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("explorer");
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        c.arg(arg);
        c
    } else {
        // The FreeDesktop file manager interface selects; else open the folder.
        let uri = format!("file://{}", path.display());
        let mut c = Command::new("dbus-send");
        c.args([
            "--session",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{uri}"))
        .arg("string:");
        if spawn(&mut c).is_ok() {
            return Ok(());
        }
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    spawn(&mut cmd)
}

fn spawn(cmd: &mut Command) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Keeps the computer from sleeping while held (idle sleep only: closing
/// a laptop's lid still sleeps).
pub struct Awake {
    #[allow(dead_code)]
    child: Option<Child>,
}

impl Awake {
    pub fn hold() -> Awake {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
            // SAFETY: a plain flag call; it holds until changed from this thread.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            Awake { child: None }
        }
        #[cfg(not(windows))]
        {
            let pid = std::process::id().to_string();
            let mut cmd = if cfg!(target_os = "macos") {
                let mut c = Command::new("caffeinate");
                c.args(["-i", "-w", &pid]);
                c
            } else {
                let mut c = Command::new("systemd-inhibit");
                c.args(["--what=sleep:idle", "--who=Orchard Download Manager", "--why=Downloading", "--mode=block", "sleep", "infinity"]);
                c
            };
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            Awake { child: cmd.spawn().ok() }
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
            // SAFETY: as above.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// A system notification. It never takes the focus. Sent from a thread of
/// its own: the notification service may take a moment to answer.
pub fn notify(summary: String, body: String) {
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.appname("Orchard Download Manager").summary(&summary);
        // Linux and the BSDs: the body may hold markup, and the icon is the
        // installed app's.
        #[cfg(all(unix, not(target_os = "macos")))]
        n.body(&escape_markup(&body)).icon("orchard-download-manager");
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        n.body(&body);
        if let Err(e) = n.show() {
            log::warn!("notification: {e}");
        }
    });
}

#[cfg(all(unix, not(target_os = "macos")))]
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Put the computer to sleep.
pub fn sleep_now() -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("pmset");
        c.arg("sleepnow");
        c
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32.exe");
        c.args(["powrprof.dll,SetSuspendState", "0,1,0"]);
        c
    } else {
        let mut c = Command::new("systemctl");
        c.arg("suspend");
        c
    };
    spawn(&mut cmd)
}
