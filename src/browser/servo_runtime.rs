use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use dpi::PhysicalSize;
use rustls::crypto::CryptoProvider;
use servo::{
    AuthenticationRequest, Code, DeviceIntPoint, DeviceIntRect, DeviceIntSize, DevicePoint,
    EventLoopWaker, InputEvent, Key as ServoKey, KeyState, KeyboardEvent as ServoKeyboardEvent,
    LoadStatus, Location, Modifiers as ServoModifiers, MouseButton, MouseButtonAction,
    MouseButtonEvent, MouseMoveEvent, NamedKey, Preferences, RenderingContext, ServoBuilder,
    ServoDelegate, ServoError, SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
    WebViewPoint, WheelDelta, WheelEvent, WheelMode,
};
use url::Url;

use crate::cli::Cli;
use crate::gfx::{Rect, Size};
use crate::input::{self, Event, Key, TerminalEvent, listen};
use crate::output::{RenderThread, Window};
use crate::ui::navigation::NavigationAction;
use crate::utils::log;

pub type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, Default)]
struct BrowserPoint {
    x: f32,
    y: f32,
}

impl BrowserPoint {
    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn to_webview_point(self) -> WebViewPoint {
        WebViewPoint::Device(DevicePoint::new(self.x, self.y))
    }
}

enum BrowserCommand {
    GoTo(String),
    GoBack,
    GoForward,
    Refresh,
    Scroll {
        delta_pixels: f64,
        point: BrowserPoint,
    },
    KeyPress(Key),
    MouseDown(BrowserPoint),
    MouseUp(BrowserPoint),
    MouseMove(BrowserPoint),
}

enum RuntimeEvent {
    Browser(BrowserCommand),
    Wake,
    Exit,
}

#[derive(Clone)]
struct ChannelWaker {
    tx: mpsc::Sender<RuntimeEvent>,
}

impl EventLoopWaker for ChannelWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.tx.send(RuntimeEvent::Wake);
    }
}

struct SharedUi {
    renderer: Mutex<RenderThread>,
    window: Mutex<Window>,
    pointer: Mutex<BrowserPoint>,
    signal_tx: mpsc::Sender<RuntimeEvent>,
    pending_frame: AtomicBool,
    animating: AtomicBool,
    running: AtomicBool,
}

impl SharedUi {
    fn new(signal_tx: mpsc::Sender<RuntimeEvent>) -> Self {
        let window = Window::read();
        let mut renderer = RenderThread::new();

        renderer.enable();
        renderer.render({
            let cells = window.cells;
            move |renderer| renderer.set_size(cells)
        });

        Self {
            renderer: Mutex::new(renderer),
            window: Mutex::new(window),
            pointer: Mutex::new(BrowserPoint::default()),
            signal_tx,
            pending_frame: AtomicBool::new(true),
            animating: AtomicBool::new(false),
            running: AtomicBool::new(true),
        }
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.signal_tx.send(RuntimeEvent::Exit);
    }

    fn window_snapshot(&self) -> Window {
        self.window.lock().unwrap().clone()
    }

    fn update_window_if_needed(&self) -> Option<Window> {
        let next = Window::read();
        let mut current = self.window.lock().unwrap();

        let changed = current.cells != next.cells
            || current.browser != next.browser
            || current.scale != next.scale
            || (current.dpi - next.dpi).abs() > f32::EPSILON;

        if !changed {
            return None;
        }

        *current = next.clone();

        self.renderer.lock().unwrap().render({
            let cells = next.cells;
            move |renderer| renderer.set_size(cells)
        });

        Some(next)
    }

    fn set_pointer(&self, point: BrowserPoint) {
        *self.pointer.lock().unwrap() = point;
    }

    fn pointer(&self) -> BrowserPoint {
        *self.pointer.lock().unwrap()
    }

    fn set_title(&self, title: Option<String>) {
        let title = title.unwrap_or_else(|| "Carboxyl".to_owned());

        self.renderer
            .lock()
            .unwrap()
            .render(move |renderer| renderer.set_title(&title).unwrap());
    }

    fn push_nav(&self, url: String, can_go_back: bool, can_go_forward: bool) {
        self.renderer
            .lock()
            .unwrap()
            .render(move |renderer| renderer.push_nav(&url, can_go_back, can_go_forward));
    }

    fn request_repaint(&self) {
        self.pending_frame.store(true, Ordering::SeqCst);
        let _ = self.signal_tx.send(RuntimeEvent::Wake);
    }

    fn take_pending_frame(&self) -> bool {
        self.pending_frame.swap(false, Ordering::SeqCst)
    }

    fn set_animating(&self, animating: bool) {
        self.animating.store(animating, Ordering::SeqCst);

        if animating {
            let _ = self.signal_tx.send(RuntimeEvent::Wake);
        }
    }

    fn is_animating(&self) -> bool {
        self.animating.load(Ordering::SeqCst)
    }
}

struct TerminalWebViewDelegate {
    ui: Arc<SharedUi>,
}

impl TerminalWebViewDelegate {
    fn new(ui: Arc<SharedUi>) -> Self {
        Self { ui }
    }
}

impl WebViewDelegate for TerminalWebViewDelegate {
    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        self.ui.push_nav(url.to_string(), false, false);
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        self.ui.set_title(title);
    }

    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.ui.request_repaint();
    }

    fn notify_history_changed(&self, _webview: WebView, entries: Vec<Url>, current: usize) {
        let Some(url) = entries.get(current) else {
            return;
        };

        self.ui
            .push_nav(url.to_string(), current > 0, current + 1 < entries.len());
    }

    fn notify_animating_changed(&self, _webview: WebView, animating: bool) {
        self.ui.set_animating(animating);
    }

    fn notify_closed(&self, _webview: WebView) {
        self.ui.stop();
    }

    fn request_authentication(&self, _webview: WebView, request: AuthenticationRequest) {
        let scope = if request.for_proxy() {
            "proxy"
        } else {
            "origin"
        };
        log::warning!(
            "authentication requested for {} ({scope}); no prompt is implemented, denying",
            request.url()
        );
    }

    fn notify_load_status_changed(&self, _webview: WebView, _status: LoadStatus) {}
}

struct TerminalServoDelegate;

impl ServoDelegate for TerminalServoDelegate {
    fn notify_error(&self, error: ServoError) {
        log::error!("servo error: {error:?}");
    }
}

struct StderrGuard {
    saved_stderr: RawFd,
    sink: File,
    log_path: Option<PathBuf>,
}

impl StderrGuard {
    fn redirect(debug: bool) -> io::Result<Self> {
        let saved_stderr = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved_stderr < 0 {
            return Err(io::Error::last_os_error());
        }

        let sink_result = if debug {
            let path =
                std::env::temp_dir().join(format!("carboxyl-stderr-{}.log", std::process::id()));
            OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)
                .map(|sink| (sink, Some(path)))
        } else {
            OpenOptions::new()
                .write(true)
                .open("/dev/null")
                .map(|sink| (sink, None))
        };

        let (sink, log_path) = match sink_result {
            Ok(result) => result,
            Err(error) => {
                unsafe {
                    libc::close(saved_stderr);
                }

                return Err(error);
            }
        };

        if unsafe { libc::dup2(sink.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::close(saved_stderr);
            }
            return Err(error);
        }

        Ok(Self {
            saved_stderr,
            sink,
            log_path,
        })
    }
}

impl Drop for StderrGuard {
    fn drop(&mut self) {
        let _ = io::Write::flush(&mut io::stderr());

        if unsafe { libc::dup2(self.saved_stderr, libc::STDERR_FILENO) } < 0 {
            return;
        }

        unsafe {
            libc::close(self.saved_stderr);
        }

        if let Some(path) = &self.log_path
            && self
                .sink
                .metadata()
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false)
        {
            eprintln!("carboxyl runtime logs were written to {}", path.display());
        }
    }
}

pub fn run(cli: Cli) -> AppResult<()> {
    let _stderr = StderrGuard::redirect(cli.debug)?;
    let _terminal = input::Terminal::setup();
    let (signal_tx, signal_rx) = mpsc::channel();
    let ui = Arc::new(SharedUi::new(signal_tx.clone()));

    ensure_rustls_provider_installed();

    let servo = ServoBuilder::default()
        .preferences(browser_preferences(Preferences::default()))
        .event_loop_waker(Box::new(ChannelWaker {
            tx: signal_tx.clone(),
        }))
        .build();
    servo.set_delegate(Rc::new(TerminalServoDelegate));

    servo.setup_logging();

    let url = normalize_url(cli.url.clone())?;
    let window = ui.window_snapshot();
    let rendering_context: Rc<dyn RenderingContext> = Rc::new(
        SoftwareRenderingContext::new(physical_size(window.browser)).map_err(|error| {
            std::io::Error::other(format!(
                "failed to create Servo software rendering context: {error:?}"
            ))
        })?,
    );
    let delegate: Rc<dyn WebViewDelegate> = Rc::new(TerminalWebViewDelegate::new(ui.clone()));
    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate)
        .url(url)
        .build();

    webview.show();
    webview.focus();
    ui.request_repaint();

    let _input = spawn_input_thread(ui.clone());

    event_loop(&servo, &webview, rendering_context, ui, signal_rx)
}

fn event_loop(
    servo: &servo::Servo,
    webview: &WebView,
    rendering_context: Rc<dyn RenderingContext>,
    ui: Arc<SharedUi>,
    signal_rx: mpsc::Receiver<RuntimeEvent>,
) -> AppResult<()> {
    while ui.is_running() {
        let window_changed = if let Some(window) = ui.update_window_if_needed() {
            let size = physical_size(window.browser);

            rendering_context.resize(size);
            webview.resize(size);
            ui.request_repaint();
            true
        } else {
            false
        };

        let timeout = if ui.is_animating() {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(250)
        };

        let should_spin = match signal_rx.recv_timeout(timeout) {
            Ok(RuntimeEvent::Browser(command)) => {
                dispatch_browser_command(webview, command)?;

                while let Ok(RuntimeEvent::Browser(command)) = signal_rx.try_recv() {
                    dispatch_browser_command(webview, command)?;
                }

                true
            }
            Ok(RuntimeEvent::Wake) => true,
            Ok(RuntimeEvent::Exit) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => ui.is_animating(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        if window_changed || should_spin {
            servo.spin_event_loop();
        }

        if ui.take_pending_frame() {
            paint_webview(webview, rendering_context.as_ref(), &ui)?;
        }
    }

    Ok(())
}

fn paint_webview(
    webview: &WebView,
    rendering_context: &dyn RenderingContext,
    ui: &Arc<SharedUi>,
) -> AppResult<()> {
    let size = ui.window_snapshot().browser;
    let rect = DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(size.width as i32, size.height as i32),
    );

    rendering_context.make_current().map_err(|error| {
        std::io::Error::other(format!(
            "failed to make Servo rendering context current: {error:?}"
        ))
    })?;

    webview.paint();

    let image = rendering_context.read_to_image(rect);
    rendering_context.present();

    let Some(image) = image else {
        return Ok(());
    };

    let width = image.width();
    let height = image.height();
    let pixels = image.into_raw();

    ui.renderer.lock().unwrap().render(move |renderer| {
        renderer.draw_background(
            &pixels,
            Size::new(width, height),
            Rect::new(0, 0, width, height),
        );
    });

    Ok(())
}

fn dispatch_browser_command(webview: &WebView, command: BrowserCommand) -> AppResult<()> {
    match command {
        BrowserCommand::GoTo(url) => webview.load(normalize_url(Some(url))?),
        BrowserCommand::GoBack => {
            if webview.can_go_back() {
                webview.go_back(1);
            }
        }
        BrowserCommand::GoForward => {
            if webview.can_go_forward() {
                webview.go_forward(1);
            }
        }
        BrowserCommand::Refresh => webview.reload(),
        BrowserCommand::Scroll {
            delta_pixels,
            point,
        } => {
            webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: 0.0,
                    y: delta_pixels,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                point.to_webview_point(),
            )));
        }
        BrowserCommand::KeyPress(key) => {
            if let Some((down, up)) = map_keyboard_event(&key) {
                webview.notify_input_event(InputEvent::Keyboard(down));
                webview.notify_input_event(InputEvent::Keyboard(up));
            }
        }
        BrowserCommand::MouseDown(point) => {
            webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Down,
                MouseButton::Left,
                point.to_webview_point(),
            )));
        }
        BrowserCommand::MouseUp(point) => {
            webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Up,
                MouseButton::Left,
                point.to_webview_point(),
            )));
        }
        BrowserCommand::MouseMove(point) => {
            webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                point.to_webview_point(),
            )));
        }
    }

    Ok(())
}

fn spawn_input_thread(ui: Arc<SharedUi>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let result = listen(|events| {
            let snapshot = ui.window_snapshot();
            let ui_for_render = ui.clone();
            let signal_tx = ui.signal_tx.clone();

            ui.renderer.lock().unwrap().render(move |renderer| {
                for event in events.iter().cloned() {
                    match event {
                        Event::Scroll { delta } => {
                            let point = ui_for_render.pointer();
                            let delta_pixels = delta as f64 * snapshot.scale.height as f64;

                            let _ = signal_tx.send(RuntimeEvent::Browser(BrowserCommand::Scroll {
                                delta_pixels,
                                point,
                            }));
                        }
                        Event::KeyPress { key } => {
                            if forward_navigation_action(
                                renderer.keypress(&key).unwrap(),
                                &signal_tx,
                            ) {
                                let _ = signal_tx
                                    .send(RuntimeEvent::Browser(BrowserCommand::KeyPress(key)));
                            }
                        }
                        Event::MouseDown { row, col } => {
                            if forward_navigation_action(
                                renderer.mouse_down((col as _, row as _).into()).unwrap(),
                                &signal_tx,
                            ) {
                                let point = scale_mouse_point(&snapshot, col, row);

                                ui_for_render.set_pointer(point);
                                let _ = signal_tx
                                    .send(RuntimeEvent::Browser(BrowserCommand::MouseDown(point)));
                            }
                        }
                        Event::MouseUp { row, col } => {
                            if forward_navigation_action(
                                renderer.mouse_up((col as _, row as _).into()).unwrap(),
                                &signal_tx,
                            ) {
                                let point = scale_mouse_point(&snapshot, col, row);

                                ui_for_render.set_pointer(point);
                                let _ = signal_tx
                                    .send(RuntimeEvent::Browser(BrowserCommand::MouseUp(point)));
                            }
                        }
                        Event::MouseMove { row, col } => {
                            if forward_navigation_action(
                                renderer.mouse_move((col as _, row as _).into()).unwrap(),
                                &signal_tx,
                            ) {
                                let point = scale_mouse_point(&snapshot, col, row);

                                ui_for_render.set_pointer(point);
                                let _ = signal_tx
                                    .send(RuntimeEvent::Browser(BrowserCommand::MouseMove(point)));
                            }
                        }
                        Event::Terminal(terminal_event) => match terminal_event {
                            TerminalEvent::Name(name) => log::debug!("terminal name: {name}"),
                            TerminalEvent::TrueColorSupported => renderer.enable_true_color(),
                        },
                        Event::Exit => {}
                    }
                }
            });
        });

        if let Err(error) = result {
            log::error!("terminal input failed: {error}");
        }

        ui.stop();
    })
}

fn forward_navigation_action(
    action: NavigationAction,
    signal_tx: &mpsc::Sender<RuntimeEvent>,
) -> bool {
    match action {
        NavigationAction::Ignore => false,
        NavigationAction::Forward => true,
        NavigationAction::GoBack() => {
            let _ = signal_tx.send(RuntimeEvent::Browser(BrowserCommand::GoBack));
            false
        }
        NavigationAction::GoForward() => {
            let _ = signal_tx.send(RuntimeEvent::Browser(BrowserCommand::GoForward));
            false
        }
        NavigationAction::Refresh() => {
            let _ = signal_tx.send(RuntimeEvent::Browser(BrowserCommand::Refresh));
            false
        }
        NavigationAction::GoTo(url) => {
            let _ = signal_tx.send(RuntimeEvent::Browser(BrowserCommand::GoTo(url)));
            false
        }
    }
}

fn scale_mouse_point(window: &Window, col: usize, row: usize) -> BrowserPoint {
    BrowserPoint::new(
        ((col as f32 + 0.5) * window.scale.width).floor(),
        ((row as f32 - 0.5) * window.scale.height).floor(),
    )
}

fn physical_size(size: Size) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width, size.height)
}

fn browser_preferences(mut preferences: Preferences) -> Preferences {
    // Servo imports proxy settings from the ambient shell environment by default.
    // This embedder does not surface proxy auth or tunnel UX yet, so inherited proxy
    // config frequently turns ordinary HTTPS navigations into opaque neterror pages.
    preferences.network_http_proxy_uri.clear();
    preferences.network_https_proxy_uri.clear();
    preferences.network_http_no_proxy.clear();
    preferences
}

fn ensure_rustls_provider_installed() {
    if CryptoProvider::get_default().is_some() {
        return;
    }

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

fn normalize_url(raw: Option<String>) -> AppResult<Url> {
    let Some(raw) = raw else {
        return Ok(Url::parse("about:blank")?);
    };

    if raw.contains("://") || raw.starts_with("about:") {
        return Ok(Url::parse(&raw)?);
    }

    if Path::new(&raw).exists() {
        return Url::from_file_path(Path::new(&raw)).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("failed to convert path into file URL: {raw}"),
            )
            .into()
        });
    }

    Ok(Url::parse(&format!("https://{raw}"))?)
}

fn map_keyboard_event(key: &Key) -> Option<(ServoKeyboardEvent, ServoKeyboardEvent)> {
    let (logical_key, code, mut modifiers) = map_logical_key_and_code(key)?;

    modifiers |= modifiers_from_input(&key.modifiers);

    Some((
        ServoKeyboardEvent::new_without_event(
            KeyState::Down,
            logical_key.clone(),
            code,
            Location::Standard,
            modifiers,
            false,
            false,
        ),
        ServoKeyboardEvent::new_without_event(
            KeyState::Up,
            logical_key,
            code,
            Location::Standard,
            modifiers,
            false,
            false,
        ),
    ))
}

fn map_logical_key_and_code(key: &Key) -> Option<(ServoKey, Code, ServoModifiers)> {
    let (char_code, forced_control) = match key.char {
        0x01..=0x1a => (key.char + b'a' - 1, true),
        value => (value, false),
    };

    let modifiers = if forced_control {
        ServoModifiers::CONTROL
    } else {
        ServoModifiers::empty()
    };

    match char_code {
        0x09 => Some((ServoKey::Named(NamedKey::Tab), Code::Tab, modifiers)),
        0x0a | 0x0d => Some((ServoKey::Named(NamedKey::Enter), Code::Enter, modifiers)),
        0x11 => Some((ServoKey::Named(NamedKey::ArrowUp), Code::ArrowUp, modifiers)),
        0x12 => Some((
            ServoKey::Named(NamedKey::ArrowDown),
            Code::ArrowDown,
            modifiers,
        )),
        0x13 => Some((
            ServoKey::Named(NamedKey::ArrowRight),
            Code::ArrowRight,
            modifiers,
        )),
        0x14 => Some((
            ServoKey::Named(NamedKey::ArrowLeft),
            Code::ArrowLeft,
            modifiers,
        )),
        0x1b => Some((ServoKey::Named(NamedKey::Escape), Code::Escape, modifiers)),
        0x20 => Some((ServoKey::Character(" ".into()), Code::Space, modifiers)),
        0x7f => Some((
            ServoKey::Named(NamedKey::Backspace),
            Code::Backspace,
            modifiers,
        )),
        value if value.is_ascii() => {
            let ch = value as char;
            let code = character_code(ch)?;

            Some((ServoKey::Character(ch.to_string()), code, modifiers))
        }
        _ => None,
    }
}

fn character_code(ch: char) -> Option<Code> {
    Some(match ch.to_ascii_lowercase() {
        'a' => Code::KeyA,
        'b' => Code::KeyB,
        'c' => Code::KeyC,
        'd' => Code::KeyD,
        'e' => Code::KeyE,
        'f' => Code::KeyF,
        'g' => Code::KeyG,
        'h' => Code::KeyH,
        'i' => Code::KeyI,
        'j' => Code::KeyJ,
        'k' => Code::KeyK,
        'l' => Code::KeyL,
        'm' => Code::KeyM,
        'n' => Code::KeyN,
        'o' => Code::KeyO,
        'p' => Code::KeyP,
        'q' => Code::KeyQ,
        'r' => Code::KeyR,
        's' => Code::KeyS,
        't' => Code::KeyT,
        'u' => Code::KeyU,
        'v' => Code::KeyV,
        'w' => Code::KeyW,
        'x' => Code::KeyX,
        'y' => Code::KeyY,
        'z' => Code::KeyZ,
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        '-' => Code::Minus,
        '=' => Code::Equal,
        '[' => Code::BracketLeft,
        ']' => Code::BracketRight,
        '\\' => Code::Backslash,
        ';' => Code::Semicolon,
        '\'' => Code::Quote,
        ',' => Code::Comma,
        '.' => Code::Period,
        '/' => Code::Slash,
        '`' => Code::Backquote,
        _ => return None,
    })
}

fn modifiers_from_input(modifiers: &crate::input::KeyModifiers) -> ServoModifiers {
    let mut mapped = ServoModifiers::empty();

    if modifiers.alt {
        mapped |= ServoModifiers::ALT;
    }
    if modifiers.control {
        mapped |= ServoModifiers::CONTROL;
    }
    if modifiers.meta {
        mapped |= ServoModifiers::META;
    }
    if modifiers.shift {
        mapped |= ServoModifiers::SHIFT;
    }

    mapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_preferences_clear_proxy_settings() {
        let preferences = browser_preferences(Preferences {
            network_http_proxy_uri: "http://proxy.example:3128".into(),
            network_https_proxy_uri: "http://proxy.example:3128".into(),
            network_http_no_proxy: "localhost,127.0.0.1".into(),
            ..Preferences::default()
        });

        assert!(preferences.network_http_proxy_uri.is_empty());
        assert!(preferences.network_https_proxy_uri.is_empty());
        assert!(preferences.network_http_no_proxy.is_empty());
    }

    #[test]
    fn rustls_provider_install_is_idempotent() {
        ensure_rustls_provider_installed();
        ensure_rustls_provider_installed();

        assert!(CryptoProvider::get_default().is_some());
    }
}
