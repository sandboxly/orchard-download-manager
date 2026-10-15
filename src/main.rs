//! The window: winit event loop, GPU surface, input conversion, wake-ups
//! from workers and downloads. Everything else is in `App`.

// Windows: a GUI program, no console window behind it. Output still reaches
// a redirected stdout; in a console window the flags that print attach to it
// (`attach_parent_console`).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orchard_download_manager::App;
use orchard_download_manager::ui;
use orchard_grid::winit;
use orchard_grid::{FontConfig, Frame, Rendered, WindowRenderer};
use orchard_kit::input::{KeyEvent, MouseEvent, MouseKind};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Theme, UserAttentionType, Window, WindowId};

mod screenshot;

#[derive(Debug, Clone, Copy)]
enum UserEvent {
    Wake,
}

struct Win {
    window: Arc<Window>,
    gpu: WindowRenderer,
    frame: Frame,
    font: FontConfig,
    title: String,
    focused: bool,
    /// The status icons the renderer has.
    icons: std::collections::HashSet<orchard_grid::ImageId>,
}

struct Platform {
    app: App,
    win: Option<Win>,
    mods: ModifiersState,
    cursor: PhysicalPosition<f64>,
    button_down: bool,
    last_click: Option<(Instant, (u16, u16))>,
    scroll_px: f64,
    /// Links from the command line, for the Add dialog once the window is up.
    startup: Vec<String>,
    /// When the next timed frame is due. Kept across wake-ups, so a stream
    /// of other events (mouse moves) can't keep pushing it back.
    next_frame: Option<Instant>,
    /// `ORCHARD_DOWNLOAD_MANAGER_EXIT_AFTER_START=1`: quit after the first
    /// frame (CI starts the real app on every platform this way).
    exit_after_start: bool,
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

impl Platform {
    fn cell_px(&self) -> (f32, f32) {
        self.win.as_ref().map_or((8.0, 16.0), |w| {
            let m = w.gpu.renderer().metrics();
            (m.width as f32, m.height as f32)
        })
    }

    fn cell_at(&self, p: PhysicalPosition<f64>) -> (u16, u16) {
        let (cw, ch) = self.cell_px();
        let col = (p.x.max(0.0) / cw as f64) as u16;
        let row = (p.y.max(0.0) / ch as f64) as u16;
        (col.min(self.app.cols.saturating_sub(1)), row.min(self.app.rows.saturating_sub(1)))
    }

    fn mouse(&mut self, kind: MouseKind, double: bool) {
        let (col, row) = self.cell_at(self.cursor);
        let ev = MouseEvent { kind, col, row, x: self.cursor.x as f32, y: self.cursor.y as f32, double };
        self.app.handle_mouse(&ev);
    }

    /// Grid size for the window's current pixel size and font.
    fn sync_size(&mut self) {
        let Some(w) = &mut self.win else { return };
        let (cols, rows) = w.gpu.grid_size();
        if (cols, rows) != (w.frame.cols(), w.frame.rows()) {
            w.frame.resize(cols, rows);
        }
        let m = w.gpu.renderer().metrics();
        self.app.resized(cols, rows, (m.width as f32, m.height as f32));
    }

    fn redraw(&mut self) {
        if let Some(w) = &self.win {
            w.window.request_redraw();
        }
    }

    fn draw(&mut self) {
        self.next_frame = None;
        // Font changed (⌘+ / ⌘-, settings)?
        let font = self.app.theme.font.clone();
        if let Some(w) = &mut self.win
            && w.font != font
        {
            if let Err(e) = w.gpu.renderer_mut().set_font(font.clone()) {
                log::warn!("font: {e}");
            }
            w.font = font;
        }
        self.sync_size();
        self.app.tick(Instant::now());
        let Some(w) = &mut self.win else { return };
        let title = self.app.title();
        if title != w.title {
            w.window.set_title(&title);
            w.title = title;
        }
        ui::draw(&self.app, &mut w.frame);
        let r = w.gpu.renderer_mut();
        orchard_download_manager::icons::upload_missing(&w.frame, &mut w.icons, |id, width, height, rgba| {
            if let Err(e) = r.set_image(id, width, height, rgba) {
                log::warn!("icon: {e}");
            }
        });
        match w.gpu.render(&w.frame) {
            Ok(Rendered::Retry) => w.window.request_redraw(),
            Ok(_) => {}
            Err(e) => log::warn!("render: {e}"),
        }
    }

    /// The app asked to come forward or for attention.
    fn window_requests(&mut self) {
        let Some(w) = &self.win else { return };
        if std::mem::take(&mut self.app.raise) {
            w.window.set_minimized(false);
            w.window.focus_window();
        }
        if std::mem::take(&mut self.app.attention) && !w.focused {
            w.window.request_user_attention(Some(UserAttentionType::Informational));
        }
    }

    fn quit(&mut self, el: &ActiveEventLoop) {
        self.app.shutdown();
        el.exit();
    }
}

/// A key press as a [`KeyEvent`] (the kit's conversion), `None` for keys the
/// app doesn't use. The media play/pause key is one: the kit types it as Space
/// for the Music Player, which here would press buttons and type spaces.
fn convert_key(ev: &winit::event::KeyEvent, mods: ModifiersState) -> Option<KeyEvent> {
    if ev.key_without_modifiers() == WKey::Named(NamedKey::MediaPlayPause) {
        return None;
    }
    orchard_kit::winit::convert_key(ev, mods)
}

impl ApplicationHandler<UserEvent> for Platform {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.win.is_some() {
            return;
        }
        let (w, h) = self.app.settings.window_size.unwrap_or((1100, 720));
        let attrs = Window::default_attributes()
            .with_title("Orchard Download Manager")
            .with_inner_size(LogicalSize::new(w.max(560), h.max(360)))
            .with_min_inner_size(LogicalSize::new(560, 360));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("orchard-download-manager: can't open a window: {e}");
                el.exit();
                return;
            }
        };
        self.app.set_system_dark(window.theme() != Some(Theme::Light));
        let font = self.app.theme.font.clone();
        let gpu = match WindowRenderer::new(window.clone(), font.clone()) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("orchard-download-manager: {e}");
                el.exit();
                return;
            }
        };
        let (cols, rows) = gpu.grid_size();
        let frame = Frame::new(cols, rows, self.app.theme.colors.background);
        self.win = Some(Win { window, gpu, frame, font, title: String::new(), focused: true, icons: Default::default() });
        self.sync_size();
        let startup = std::mem::take(&mut self.startup);
        if !startup.is_empty() {
            self.app.external_links(startup);
        }
        self.redraw();
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.redraw();
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, _: UserEvent) {
        if self.app.process() {
            self.redraw();
        }
        self.window_requests();
        if self.app.quit {
            self.quit(el);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let mut input = true;
        match event {
            WindowEvent::CloseRequested => {
                self.quit(el);
                return;
            }
            WindowEvent::RedrawRequested => {
                self.draw();
                input = false;
                if std::mem::take(&mut self.exit_after_start) {
                    println!("opened, drew a frame and closed");
                    self.quit(el);
                    return;
                }
            }
            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if let Some(w) = &mut self.win {
                    w.gpu.resize(width, height);
                    let s = PhysicalSize::new(width, height).to_logical::<u32>(w.window.scale_factor());
                    self.app.settings.window_size = Some((s.width, s.height));
                }
                self.sync_size();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(w) = &mut self.win {
                    let _ = w.gpu.set_scale_factor(scale_factor);
                }
                self.sync_size();
            }
            WindowEvent::ThemeChanged(t) => self.app.set_system_dark(t == Theme::Dark),
            // Shown again: frames drawn while it was hidden were dropped
            // (on macOS also the very first one).
            WindowEvent::Occluded(false) => {}
            WindowEvent::Focused(f) => {
                if let Some(w) = &mut self.win {
                    w.focused = f;
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
                input = false;
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(k) = convert_key(&event, self.mods) {
                    self.app.handle_key(&k);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if self.button_down {
                    self.mouse(MouseKind::Drag, false);
                } else {
                    input = false;
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                if state == ElementState::Pressed {
                    self.button_down = true;
                    let cell = self.cell_at(self.cursor);
                    let now = Instant::now();
                    let double = self.last_click.is_some_and(|(t, c)| c == cell && now.duration_since(t) < DOUBLE_CLICK);
                    self.last_click = if double { None } else { Some((now, cell)) };
                    self.mouse(MouseKind::Down, double);
                } else {
                    self.button_down = false;
                    self.mouse(MouseKind::Up, false);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.scroll_px += p.y;
                        let ch = self.cell_px().1 as f64;
                        let whole = (self.scroll_px / ch).trunc();
                        self.scroll_px -= whole * ch;
                        whole as f32
                    }
                };
                if rows != 0.0 {
                    self.mouse(MouseKind::Scroll(rows), false);
                }
            }
            _ => input = false,
        }
        if input {
            self.window_requests();
            if self.app.quit {
                self.quit(el);
                return;
            }
            self.redraw();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.app.process() {
            self.redraw();
        }
        self.window_requests();
        if self.app.quit {
            self.quit(el);
            return;
        }
        let now = Instant::now();
        if self.next_frame.is_some_and(|t| now >= t) {
            self.next_frame = None;
            self.redraw();
        }
        if self.next_frame.is_none() {
            self.next_frame = self.app.next_frame(now);
        }
        el.set_control_flow(match self.next_frame {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        // ⌘Q from the macOS app menu ends the loop without CloseRequested.
        self.app.shutdown();
    }
}

/// Panics also go to `crash.log` in the config folder: on Windows the app
/// has no console, so a crash would leave no trace otherwise.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = orchard_download_manager::settings::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let text = format!("Orchard Download Manager {} crashed: {info}\n{backtrace}\n", env!("CARGO_PKG_VERSION"));
        let _ = std::fs::write(dir.join("crash.log"), text);
        default(info);
    }));
}

fn usage() -> &'static str {
    "usage: orchard-download-manager [LINK]...\n       orchard-download-manager --version | --self-test\n       orchard-download-manager --screenshot OUT.png [--size COLSxROWS] [--theme NAME] [--dialog add|settings[:PAGE]|remove|properties|checksum|help] [--empty]"
}

/// `--self-test`: downloading and drawing work in this build (for
/// packaging checks; no window).
fn self_test() -> ExitCode {
    println!("Orchard Download Manager {}", env!("CARGO_PKG_VERSION"));
    let dir = std::env::temp_dir();
    let shot = dir.join(format!("orchard-download-manager-self-test-{}.png", std::process::id()));
    let draw = match screenshot::run(&[shot.to_string_lossy().into_owned()]) {
        ExitCode::SUCCESS => Ok("the UI drew headlessly".to_string()),
        _ => Err("the UI didn't draw".to_string()),
    };
    let _ = std::fs::remove_file(&shot);
    let net = orchard_download_manager::engine::http::agent(&Default::default()).map(|_| "the HTTPS client starts (system certificates)".to_string());
    let mut ok = true;
    for (what, result) in [("render", draw), ("network", net)] {
        match result {
            Ok(done) => println!("ok    {what}: {done}"),
            Err(e) => {
                ok = false;
                println!("FAIL  {what}: {e}");
            }
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Windows: started with an option (`--self-test`, `--version`…) from a
/// console and with nothing redirected, print to that console.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    if !std::env::args_os().skip(1).any(|a| a.to_string_lossy().starts_with('-')) {
        return;
    }
    // SAFETY: plain Win32 calls without pointers.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() -> ExitCode {
    #[cfg(windows)]
    attach_parent_console();
    log_panics();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Started by the browser for its extension: talk to it, no window.
    if orchard_download_manager::native_host::is_invocation(&args) {
        return orchard_download_manager::native_host::run();
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{}", usage());
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("Orchard Download Manager {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--self-test") {
        return self_test();
    }
    if args.first().map(String::as_str) == Some("--screenshot") {
        return screenshot::run(&args[1..]);
    }
    // macOS passes -psn_… to apps started from the Finder.
    let links: Vec<String> = args.into_iter().filter(|a| !a.starts_with('-')).collect();

    // Already running: hand the links over (or just bring it forward).
    let config = orchard_download_manager::settings::config_dir();
    if orchard_download_manager::ipc::send(&config, &links) {
        return ExitCode::SUCCESS;
    }

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("orchard-download-manager: {e}");
            return ExitCode::FAILURE;
        }
    };
    let proxy = Mutex::new(event_loop.create_proxy());
    let wake: orchard_kit::tasks::Waker = Arc::new(move || {
        let _ = proxy.lock().unwrap().send_event(UserEvent::Wake);
    });
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
    let mut platform = Platform {
        app,
        win: None,
        mods: ModifiersState::empty(),
        cursor: PhysicalPosition::new(0.0, 0.0),
        button_down: false,
        last_click: None,
        scroll_px: 0.0,
        startup: links,
        next_frame: None,
        exit_after_start: std::env::var_os("ORCHARD_DOWNLOAD_MANAGER_EXIT_AFTER_START").is_some_and(|v| v == "1"),
    };
    match event_loop.run_app(&mut platform) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("orchard-download-manager: {e}");
            ExitCode::FAILURE
        }
    }
}
