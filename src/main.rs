//! The window: the shared shell (`orchard_shell`: winit event loop, GPU
//! surface, input conversion, wake-ups from workers and downloads) around
//! `App`, plus what only this app does there. Everything else is in `App`.

// Windows: a GUI program, no console window behind it. Output still reaches
// a redirected stdout; in a console window the flags that print attach to it
// (`orchard_shell::cli::attach_parent_console`).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::process::ExitCode;
use std::time::Instant;

use orchard_download_manager::App;
use orchard_download_manager::ui;
use orchard_grid::{FontConfig, Frame, GridRenderer, ImageId, Rgba};
use orchard_kit::input::{KeyEvent, MouseEvent};
use orchard_shell::winit::window::{UserAttentionType, Window};
use orchard_shell::{AppInfo, ShellApp, cli};

mod screenshot;

const INFO: AppInfo = AppInfo {
    name: "Orchard Download Manager",
    bin: "orchard-download-manager",
    version: env!("CARGO_PKG_VERSION"),
    default_size: (1100, 720),
    min_size: (560, 360),
    exit_after_start_env: "ORCHARD_DOWNLOAD_MANAGER_EXIT_AFTER_START",
};

/// The app in the shell's window.
struct Shell {
    app: App,
    /// The window title shown.
    title: String,
    /// The status icons the renderer has.
    icons: HashSet<ImageId>,
    /// Links from the command line, for the Add dialog once the window is up.
    startup: Vec<String>,
}

impl ShellApp for Shell {
    const QUIT_AFTER_WAIT: bool = true;

    fn font(&self) -> &FontConfig {
        &self.app.theme.font
    }

    fn background(&self) -> Rgba {
        self.app.theme.colors.background
    }

    fn grid(&self) -> (u16, u16) {
        (self.app.cols, self.app.rows)
    }

    fn window_size(&self) -> Option<(u32, u32)> {
        self.app.settings.window_size
    }

    fn set_window_size(&mut self, logical: (u32, u32)) {
        self.app.settings.window_size = Some(logical);
    }

    fn quit_requested(&self) -> bool {
        self.app.quit
    }

    fn set_system_dark(&mut self, dark: bool) {
        self.app.set_system_dark(dark);
    }

    fn resized(&mut self, cols: u16, rows: u16, cell_px: (f32, f32)) {
        self.app.resized(cols, rows, cell_px);
    }

    fn tick(&mut self, now: Instant) {
        self.app.tick(now);
    }

    fn draw(&self, frame: &mut Frame) {
        ui::draw(&self.app, frame);
    }

    fn process(&mut self) -> bool {
        self.app.process()
    }

    fn next_frame(&self, now: Instant) -> Option<Instant> {
        self.app.next_frame(now)
    }

    fn handle_key(&mut self, ev: &KeyEvent) {
        self.app.handle_key(ev);
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) {
        self.app.handle_mouse(ev);
    }

    fn save(&mut self) {
        self.app.shutdown();
    }

    fn started(&mut self, _window: &Window) {
        let startup = std::mem::take(&mut self.startup);
        if !startup.is_empty() {
            self.app.external_links(startup);
        }
    }

    fn before_draw(&mut self, window: &Window, _renderer: &mut GridRenderer) {
        let title = self.app.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
    }

    fn after_draw(&mut self, frame: &Frame, renderer: &mut GridRenderer) {
        orchard_download_manager::icons::upload_missing(frame, &mut self.icons, |id, width, height, rgba| {
            if let Err(e) = renderer.set_image(id, width, height, rgba) {
                log::warn!("icon: {e}");
            }
        });
    }

    /// The app asked to come forward or for attention.
    fn window_requests(&mut self, window: &Window, focused: bool) {
        if std::mem::take(&mut self.app.raise) {
            window.set_minimized(false);
            window.focus_window();
        }
        if std::mem::take(&mut self.app.attention) && !focused {
            window.request_user_attention(Some(UserAttentionType::Informational));
        }
    }

    fn focus_changed(&mut self, _focused: bool) -> bool {
        true
    }
}

fn usage() -> &'static str {
    "usage: orchard-download-manager [LINK]...\n       orchard-download-manager --version | --self-test\n       orchard-download-manager --screenshot OUT.png [--size COLSxROWS] [--theme NAME] [--dialog add|batch|settings[:PAGE]|remove|properties|checksum|help] [--picker|--picker-menu] [--empty] [--cursor N]"
}

/// `--self-test`: downloading and drawing work in this build (for
/// packaging checks; no window).
fn self_test() -> ExitCode {
    cli::self_test(
        &INFO,
        vec![
            ("render", Box::new(|| cli::render_check(&INFO, screenshot::run))),
            (
                "network",
                Box::new(|| {
                    orchard_download_manager::engine::http::agent(&Default::default())
                        .map(|_| "the HTTPS client starts (system certificates)".to_string())
                }),
            ),
        ],
    )
}

fn main() -> ExitCode {
    #[cfg(windows)]
    cli::attach_parent_console();
    cli::log_panics(&INFO, orchard_download_manager::settings::config_dir);
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Started by the browser for its extension: talk to it, no window.
    if orchard_download_manager::native_host::is_invocation(&args) {
        return orchard_download_manager::native_host::run();
    }
    if let Some(code) = cli::common_args(&INFO, &args, usage(), self_test, screenshot::run) {
        return code;
    }
    // macOS passes -psn_… to apps started from the Finder.
    let links: Vec<String> = args.into_iter().filter(|a| !a.starts_with('-')).collect();

    // Already running: hand the links over (or just bring it forward).
    let config = orchard_download_manager::settings::config_dir();
    if orchard_download_manager::ipc::send(&config, &links) {
        return ExitCode::SUCCESS;
    }

    orchard_shell::run(&INFO, |wake| {
        let app = App::new(wake, true);
        // Let the browsers find the extension's helper (this copy of the app).
        // Not for a separate test setup, which the helper wouldn't reach.
        if std::env::var_os("ORCHARD_DOWNLOAD_MANAGER_CONFIG_DIR").is_none() {
            std::thread::spawn(orchard_download_manager::native_host::register);
        }
        let tasks = app.tasks.clone();
        if let Err(e) = orchard_download_manager::ipc::listen(&config, move |h| tasks.post(move |app| app.handoff(h))) {
            log::warn!("listening for other starts: {e}");
        }
        Shell { app, title: String::new(), icons: HashSet::new(), startup: links }
    })
}
