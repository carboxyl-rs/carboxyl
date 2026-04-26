use rustix::fd::IntoRawFd;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture,
    poll as ct_poll, read as ct_read,
};
use dpi::PhysicalSize;
use ratatui::layout::{Constraint, Layout};
use ratatui::{DefaultTerminal, Frame};
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
use crate::gfx::Size;
use crate::input::{self, Key, map_crossterm_event};
use crate::output::{BrowserFrame, BrowserWidget, NavAction, NavState, NavWidget, Window};
use crate::utils::log;

pub type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

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

fn scale_mouse(window: &Window, col: u16, row: u16) -> BrowserPoint {
    BrowserPoint::new(
        ((col as f32 + 0.5) * window.cell_pixels.width).floor(),
        ((row as f32 - 0.5) * window.cell_pixels.height).floor(),
    )
}

fn physical_size(size: Size<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width, size.height)
}

// ---------------------------------------------------------------------------
// Event model
// ---------------------------------------------------------------------------

enum RuntimeEvent {
    /// Normalised input event from the input thread.
    Input(input::Event),
    /// Servo needs the loop to spin.
    Wake,
    /// New frame is ready to paint.
    FrameReady,
    /// Nav/title update from a WebView delegate callback.
    Delegate(DelegateEvent),
    /// Graceful shutdown.
    Exit,
}

enum DelegateEvent {
    UrlChanged(String),
    TitleChanged(String),
    HistoryChanged {
        url: String,
        can_go_back: bool,
        can_go_forward: bool,
    },
    Closed,
}

// ---------------------------------------------------------------------------
// Servo EventLoopWaker
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ChannelWaker {
    tx: mpsc::SyncSender<RuntimeEvent>,
}

impl EventLoopWaker for ChannelWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.tx.try_send(RuntimeEvent::Wake);
    }
}

// ---------------------------------------------------------------------------
// WebView delegate — pure event emitter, no shared state
// ---------------------------------------------------------------------------

struct TerminalWebViewDelegate {
    tx: mpsc::SyncSender<RuntimeEvent>,
}

impl WebViewDelegate for TerminalWebViewDelegate {
    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        let _ = self
            .tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::UrlChanged(
                url.to_string(),
            )));
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        let title = title.unwrap_or_else(|| "Carboxyl".to_owned());
        let _ = self
            .tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::TitleChanged(title)));
    }

    fn notify_new_frame_ready(&self, _webview: WebView) {
        let _ = self.tx.try_send(RuntimeEvent::FrameReady);
    }

    fn notify_history_changed(&self, _webview: WebView, entries: Vec<Url>, current: usize) {
        let Some(url) = entries.get(current) else {
            return;
        };
        let _ = self
            .tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::HistoryChanged {
                url: url.to_string(),
                can_go_back: current > 0,
                can_go_forward: current + 1 < entries.len(),
            }));
    }

    fn notify_animating_changed(&self, _webview: WebView, animating: bool) {
        // When animating, Servo fires notify_new_frame_ready continuously,
        // keeping FrameReady events flowing. No explicit tracking needed.
        if animating {
            let _ = self.tx.try_send(RuntimeEvent::Wake);
        }
    }

    fn notify_closed(&self, _webview: WebView) {
        let _ = self
            .tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::Closed));
    }

    fn request_authentication(&self, _webview: WebView, request: AuthenticationRequest) {
        let scope = if request.for_proxy() {
            "proxy"
        } else {
            "origin"
        };
        log::warning!(
            "authentication requested for {} ({scope}); no prompt implemented, denying",
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

// ---------------------------------------------------------------------------
// stderr redirect guard
// ---------------------------------------------------------------------------

struct StderrGuard {
    saved: RawFd,
    sink: File,
    log_path: Option<PathBuf>,
}

impl StderrGuard {
    fn redirect(debug: bool) -> io::Result<Self> {
        let saved = rustix::io::dup(rustix::stdio::stderr())
            .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?
            .into_raw_fd();

        let (sink, log_path) = if debug {
            let path =
                std::env::temp_dir().join(format!("carboxyl-stderr-{}.log", std::process::id()));
            let f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)?;
            (f, Some(path))
        } else {
            (OpenOptions::new().write(true).open("/dev/null")?, None)
        };

        unsafe {
            libc_dup2(sink.as_raw_fd(), 2 /* STDERR_FILENO */)?;
        }

        Ok(Self {
            saved,
            sink,
            log_path,
        })
    }
}

impl Drop for StderrGuard {
    fn drop(&mut self) {
        let _ = io::stderr().flush();
        unsafe { libc_dup2(self.saved, 2).ok() };
        unsafe { rustix::io::close(self.saved) };

        if let Some(path) = &self.log_path
            && self.sink.metadata().map(|m| m.len() > 0).unwrap_or(false)
        {
            eprintln!("carboxyl runtime logs were written to {}", path.display());
        }
    }
}

/// `dup2` is not exposed by rustix (it's intentionally omitted as non-POSIX
/// safe), so we call it directly via libc for the stderr redirect only.
unsafe fn libc_dup2(old: RawFd, new: RawFd) -> io::Result<()> {
    if unsafe { libc::dup2(old, new) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// True-color detection
//
// We probe the terminal by querying the current color via a DCS sequence
// before ratatui takes over the screen. The response is parsed in the input
// thread and forwarded as `Event::TrueColorSupported`.
// ---------------------------------------------------------------------------

fn probe_true_color() -> io::Result<()> {
    let mut out = io::stdout();
    // Query current background color to detect true-color support.
    write!(out, "\x1bP$qm\x1b\\")?;
    out.flush()
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(cli: Cli) -> AppResult<()> {
    let _stderr = StderrGuard::redirect(cli.debug)?;

    // Probe true-color *before* ratatui initialises the terminal so the
    // response arrives while we can still read raw stdin.
    probe_true_color()?;

    ensure_rustls_provider_installed();

    let (event_tx, event_rx) = mpsc::sync_channel::<RuntimeEvent>(256);

    // Initialise ratatui (enters alt screen, enables raw mode).
    let terminal = ratatui::init();

    // Enable mouse reporting through crossterm.
    crossterm::execute!(io::stdout(), EnableMouseCapture)?;

    let window = Window::read(&cli);

    let servo = ServoBuilder::default()
        .preferences(browser_preferences(Preferences::default()))
        .event_loop_waker(Box::new(ChannelWaker {
            tx: event_tx.clone(),
        }))
        .build();

    servo.set_delegate(Rc::new(TerminalServoDelegate));
    servo.setup_logging();

    let url = normalize_url(cli.url.clone())?;
    let rendering_context: Rc<dyn RenderingContext> = Rc::new(
        SoftwareRenderingContext::new(physical_size(window.browser)).map_err(|e| {
            io::Error::other(format!(
                "failed to create Servo software rendering context: {e:?}"
            ))
        })?,
    );

    let delegate: Rc<dyn WebViewDelegate> = Rc::new(TerminalWebViewDelegate {
        tx: event_tx.clone(),
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate)
        .url(url)
        .build();

    webview.show();
    webview.focus();

    let _input = spawn_input_thread(event_tx.clone());

    let result = event_loop(
        &servo,
        &webview,
        rendering_context,
        terminal,
        window,
        &cli,
        event_rx,
    );

    // Restore terminal regardless of outcome.
    crossterm::execute!(io::stdout(), DisableMouseCapture)?;
    ratatui::restore();

    result
}

// ---------------------------------------------------------------------------
// Main event loop
// ---------------------------------------------------------------------------

fn event_loop(
    servo: &servo::Servo,
    webview: &WebView,
    rendering_context: Rc<dyn RenderingContext>,
    mut terminal: DefaultTerminal,
    mut window: Window,
    cli: &Cli,
    event_rx: mpsc::Receiver<RuntimeEvent>,
) -> AppResult<()> {
    let mut pointer = BrowserPoint::default();
    let mut nav = NavState::default();
    let mut frame: Option<BrowserFrame> = None;
    let mut true_color = false;
    let mut running = true;
    let mut pending_paint = true;

    const IDLE_TIMEOUT: Duration = Duration::from_millis(250);

    while running {
        // Detect terminal resize.
        {
            let next = Window::read(cli);
            if next.differs_from(&window) {
                rendering_context.resize(physical_size(next.browser));
                webview.resize(physical_size(next.browser));
                pending_paint = true;
                window = next;
            }
        }

        let should_spin = match event_rx.recv_timeout(IDLE_TIMEOUT) {
            Ok(RuntimeEvent::Input(event)) => {
                handle_input(
                    event,
                    webview,
                    &window,
                    &mut nav,
                    &mut pointer,
                    &mut true_color,
                    &mut running,
                )?;

                while let Ok(RuntimeEvent::Input(event)) = event_rx.try_recv() {
                    handle_input(
                        event,
                        webview,
                        &window,
                        &mut nav,
                        &mut pointer,
                        &mut true_color,
                        &mut running,
                    )?;
                }

                pending_paint = true;
                true
            }

            Ok(RuntimeEvent::Wake) => true,

            Ok(RuntimeEvent::FrameReady) => {
                paint_servo(webview, rendering_context.as_ref(), &mut frame, &window)?;
                pending_paint = true;
                // Don't spin again immediately — the frame is already painted.
                false
            }

            Ok(RuntimeEvent::Delegate(ev)) => {
                match ev {
                    DelegateEvent::UrlChanged(url) => {
                        nav.push(&url, nav.can_go_back, nav.can_go_forward);
                    }
                    DelegateEvent::TitleChanged(title) => {
                        // Set the terminal window title via an OSC sequence.
                        let _ = write!(io::stdout(), "\x1b]0;{title}\x07");
                        let _ = io::stdout().flush();
                    }
                    DelegateEvent::HistoryChanged {
                        url,
                        can_go_back,
                        can_go_forward,
                    } => {
                        nav.push(&url, can_go_back, can_go_forward);
                    }
                    DelegateEvent::Closed => {
                        running = false;
                    }
                }
                pending_paint = true;
                false
            }

            Ok(RuntimeEvent::Exit) => {
                running = false;
                false
            }

            Err(mpsc::RecvTimeoutError::Timeout) => false,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        if should_spin {
            servo.spin_event_loop();
        }

        if pending_paint {
            pending_paint = false;
            draw_frame(&mut terminal, &nav, frame.as_ref(), true_color)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Input dispatch
// ---------------------------------------------------------------------------

fn handle_input(
    event: input::Event,
    webview: &WebView,
    window: &Window,
    nav: &mut NavState,
    pointer: &mut BrowserPoint,
    true_color: &mut bool,
    running: &mut bool,
) -> AppResult<()> {
    match event {
        input::Event::Exit => *running = false,

        input::Event::TrueColorSupported => *true_color = true,

        input::Event::Scroll { delta } => {
            webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: 0.0,
                    y: delta as f64 * window.cell_pixels.height as f64,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                pointer.to_webview_point(),
            )));
        }

        input::Event::KeyPress(key) => {
            let action = nav.keypress(key.char, key.modifiers.alt, key.modifiers.meta);
            dispatch_nav_action(action, webview)?;

            // Also forward to Servo if the nav bar didn't consume it.
            if let Some((down, up)) = map_keyboard_event(&key) {
                webview.notify_input_event(InputEvent::Keyboard(down));
                webview.notify_input_event(InputEvent::Keyboard(up));
            }
        }

        input::Event::MouseDown { row, col } => {
            let action = nav.mouse_down(col, row);

            if matches!(action, NavAction::Forward) {
                let point = scale_mouse(window, col, row);
                *pointer = point;
                webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Down,
                    MouseButton::Left,
                    point.to_webview_point(),
                )));
            } else {
                dispatch_nav_action(action, webview)?;
            }
        }

        input::Event::MouseUp { row, col } => {
            let action = nav.mouse_up(col, row);

            if matches!(action, NavAction::Forward) {
                let point = scale_mouse(window, col, row);
                *pointer = point;
                webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Up,
                    MouseButton::Left,
                    point.to_webview_point(),
                )));
            } else {
                dispatch_nav_action(action, webview)?;
            }
        }

        input::Event::MouseMove { row, col } => {
            let point = scale_mouse(window, col, row);
            *pointer = point;
            webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                point.to_webview_point(),
            )));
        }
    }

    Ok(())
}

fn dispatch_nav_action(action: NavAction, webview: &WebView) -> AppResult<()> {
    match action {
        NavAction::Ignore | NavAction::Forward => {}
        NavAction::GoBack => {
            if webview.can_go_back() {
                webview.go_back(1);
            }
        }
        NavAction::GoForward => {
            if webview.can_go_forward() {
                webview.go_forward(1);
            }
        }
        NavAction::Refresh => webview.reload(),
        NavAction::GoTo(url) => webview.load(normalize_url(Some(url))?),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Pull the latest frame from Servo's rendering context into `frame`.
fn paint_servo(
    webview: &WebView,
    rendering_context: &dyn RenderingContext,
    frame: &mut Option<BrowserFrame>,
    window: &Window,
) -> AppResult<()> {
    let size = window.browser;
    let rect = DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(size.width as i32, size.height as i32),
    );

    rendering_context.make_current().map_err(|e| {
        io::Error::other(format!(
            "failed to make Servo rendering context current: {e:?}"
        ))
    })?;

    webview.paint();

    let image = rendering_context.read_to_image(rect);
    rendering_context.present();

    if let Some(img) = image {
        *frame = Some(BrowserFrame {
            pixels: img.into_raw(),
            size,
        });
    }

    Ok(())
}

/// Compose and draw a full ratatui frame.
fn draw_frame(
    terminal: &mut DefaultTerminal,
    nav: &NavState,
    frame: Option<&BrowserFrame>,
    true_color: bool,
) -> AppResult<()> {
    terminal.draw(|f: &mut Frame| {
        let area = f.area();

        // Split vertically: 1 row for nav bar, rest for browser.
        let [nav_area, browser_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);

        f.render_widget(NavWidget::new(nav), nav_area);

        if let Some(frame) = frame {
            f.render_widget(BrowserWidget::new(frame, true_color), browser_area);
        }

        // Position the terminal cursor for the URL field if focused.
        if let Some((col, row)) = NavWidget::new(nav).cursor_position(nav_area) {
            f.set_cursor_position((col, row));
        }
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Input thread — pure crossterm event translator
// ---------------------------------------------------------------------------

fn spawn_input_thread(tx: mpsc::SyncSender<RuntimeEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            // Poll with a short timeout so we don't block indefinitely and
            // miss a shutdown signal from the main loop dropping the channel.
            match ct_poll(Duration::from_millis(50)) {
                Err(e) => {
                    log::error!("crossterm poll error: {e}");
                    break;
                }
                Ok(false) => {
                    // No event yet — check if the channel is still alive.
                    if tx.try_send(RuntimeEvent::Wake).is_err() {
                        // Receiver dropped — main loop exited.
                        break;
                    }
                    // That spurious Wake is harmless; continue polling.
                    continue;
                }
                Ok(true) => {}
            }

            let ct_event = match ct_read() {
                Ok(e) => e,
                Err(e) => {
                    log::error!("crossterm read error: {e}");
                    break;
                }
            };

            for event in map_crossterm_event(ct_event) {
                let is_exit = matches!(event, input::Event::Exit);
                let send_result = tx.try_send(RuntimeEvent::Input(event));

                if send_result.is_err() || is_exit {
                    return;
                }
            }
        }

        let _ = tx.try_send(RuntimeEvent::Exit);
    })
}

// ---------------------------------------------------------------------------
// URL normalisation
// ---------------------------------------------------------------------------

fn normalize_url(raw: Option<String>) -> AppResult<Url> {
    let Some(raw) = raw else {
        return Ok(Url::parse("about:blank")?);
    };

    if raw.contains("://") || raw.starts_with("about:") {
        return Ok(Url::parse(&raw)?);
    }

    if Path::new(&raw).exists() {
        return Url::from_file_path(Path::new(&raw)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("failed to convert path into file URL: {raw}"),
            )
            .into()
        });
    }

    Ok(Url::parse(&format!("https://{raw}"))?)
}

// ---------------------------------------------------------------------------
// Servo preferences
// ---------------------------------------------------------------------------

fn browser_preferences(mut preferences: Preferences) -> Preferences {
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

// ---------------------------------------------------------------------------
// Keyboard mapping
// ---------------------------------------------------------------------------

fn map_keyboard_event(key: &Key) -> Option<(ServoKeyboardEvent, ServoKeyboardEvent)> {
    let (logical_key, code, mut modifiers) = map_logical_key(key)?;
    modifiers |= modifiers_from_key(key);

    let make = |state| {
        ServoKeyboardEvent::new_without_event(
            state,
            logical_key.clone(),
            code,
            Location::Standard,
            modifiers,
            false,
            false,
        )
    };

    Some((make(KeyState::Down), make(KeyState::Up)))
}

fn map_logical_key(key: &Key) -> Option<(ServoKey, Code, ServoModifiers)> {
    let (char_code, forced_ctrl) = match key.char {
        0x01..=0x1a => (key.char + b'a' - 1, true),
        v => (v, false),
    };

    let modifiers = if forced_ctrl {
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
        v if v.is_ascii() => {
            let ch = v as char;
            Some((
                ServoKey::Character(ch.to_string()),
                character_code(ch)?,
                modifiers,
            ))
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

fn modifiers_from_key(key: &Key) -> ServoModifiers {
    let mut m = ServoModifiers::empty();
    if key.modifiers.alt {
        m |= ServoModifiers::ALT;
    }
    if key.modifiers.ctrl {
        m |= ServoModifiers::CONTROL;
    }
    if key.modifiers.meta {
        m |= ServoModifiers::META;
    }
    if key.modifiers.shift {
        m |= ServoModifiers::SHIFT;
    }
    m
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_preferences_clears_proxy() {
        let prefs = browser_preferences(Preferences {
            network_http_proxy_uri: "http://proxy.example:3128".into(),
            network_https_proxy_uri: "http://proxy.example:3128".into(),
            network_http_no_proxy: "localhost".into(),
            ..Preferences::default()
        });

        assert!(prefs.network_http_proxy_uri.is_empty());
        assert!(prefs.network_https_proxy_uri.is_empty());
        assert!(prefs.network_http_no_proxy.is_empty());
    }

    #[test]
    fn rustls_provider_install_idempotent() {
        ensure_rustls_provider_installed();
        ensure_rustls_provider_installed();
        assert!(CryptoProvider::get_default().is_some());
    }
}
