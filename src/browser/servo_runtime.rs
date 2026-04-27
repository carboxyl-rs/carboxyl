use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, poll as ct_poll, read as ct_read,
};
use dpi::PhysicalSize;
use glam::{UVec2, Vec2};
use log::{error, warn};
use ratatui::layout::{Constraint, Layout};
use ratatui::{DefaultTerminal, Frame};
use rustls::crypto::CryptoProvider;
use servo::{
    AuthenticationRequest, Code, DeviceIntPoint, DeviceIntRect, DeviceIntSize, DevicePoint,
    EventLoopWaker, InputEvent, Key as ServoKey, KeyState,
    KeyboardEvent as ServoKeyboardEvent, LoadStatus, Location,
    Modifiers as ServoModifiers, MouseButton, MouseButtonAction, MouseButtonEvent,
    MouseMoveEvent, NamedKey, Preferences, RenderingContext, ServoBuilder, ServoDelegate,
    ServoError, SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
    WebViewPoint, WheelDelta, WheelEvent, WheelMode,
};
use simplelog::{Config, LevelFilter, WriteLogger};
use url::Url;

use crate::cli::Cli;
use crate::input::{self, Key, map_crossterm_event};
use crate::output::{BrowserFrame, BrowserWidget, NavAction, NavState, NavWidget, Window};

pub type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
struct BrowserPoint(Vec2);

impl BrowserPoint {
    fn from_cell(window: &Window, col: u16, row: u16) -> Self {
        Self(Vec2::new(
            ((col as f32 + 0.5) * window.cell_pixels.x).floor(),
            ((row as f32 - 0.5) * window.cell_pixels.y).floor(),
        ))
    }

    fn to_webview_point(self) -> WebViewPoint {
        WebViewPoint::Device(DevicePoint::new(self.0.x, self.0.y))
    }
}

fn physical_size(size: UVec2) -> PhysicalSize<u32> {
    PhysicalSize::new(size.x, size.y)
}

// ---------------------------------------------------------------------------
// Event model
// ---------------------------------------------------------------------------

/// Everything the main loop can receive.
enum RuntimeEvent {
    Input(input::Event),
    Wake,
    Frame(BrowserFrame),
    Delegate(DelegateEvent),
    Exit,
}

/// Updates originating from WebView delegate callbacks on the Servo thread.
enum DelegateEvent {
    UrlChanged(String),
    TitleChanged(String),
    HistoryChanged { url: String, can_go_back: bool, can_go_forward: bool },
    Closed,
}

/// Commands the main thread sends to the Servo thread.
enum ServoCommand {
    Load(Url),
    GoBack,
    GoForward,
    Reload,
    Resize(PhysicalSize<u32>),
    Input(InputEvent),
    Paint,
    Shutdown,
}

// ---------------------------------------------------------------------------
// Servo EventLoopWaker — wakes the Servo thread's own recv loop
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ServoWaker {
    tx: mpsc::SyncSender<ServoCommand>,
}

impl EventLoopWaker for ServoWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        // An empty paint request is enough to wake the Servo thread.
        let _ = self.tx.try_send(ServoCommand::Paint);
    }
}

// ---------------------------------------------------------------------------
// WebView delegate — pure event emitter to the main loop
// ---------------------------------------------------------------------------

struct TerminalWebViewDelegate {
    event_tx: mpsc::SyncSender<RuntimeEvent>,
}

impl WebViewDelegate for TerminalWebViewDelegate {
    fn notify_url_changed(&self, _: WebView, url: Url) {
        let _ = self.event_tx.try_send(RuntimeEvent::Delegate(
            DelegateEvent::UrlChanged(url.to_string()),
        ));
    }

    fn notify_page_title_changed(&self, _: WebView, title: Option<String>) {
        let title = title.unwrap_or_else(|| "Carboxyl".to_owned());
        let _ = self.event_tx.try_send(RuntimeEvent::Delegate(
            DelegateEvent::TitleChanged(title),
        ));
    }

    fn notify_new_frame_ready(&self, _: WebView) {
        // Signal the Servo thread to paint; it will send Frame back.
        // We don't paint inline here because the Servo thread owns the
        // rendering context.
        let _ = self.event_tx.try_send(RuntimeEvent::Wake);
    }

    fn notify_history_changed(&self, _: WebView, entries: Vec<Url>, current: usize) {
        let Some(url) = entries.get(current) else { return };
        let _ = self.event_tx.try_send(RuntimeEvent::Delegate(
            DelegateEvent::HistoryChanged {
                url: url.to_string(),
                can_go_back: current > 0,
                can_go_forward: current + 1 < entries.len(),
            },
        ));
    }

    fn notify_animating_changed(&self, _: WebView, animating: bool) {
        if animating {
            let _ = self.event_tx.try_send(RuntimeEvent::Wake);
        }
    }

    fn notify_closed(&self, _: WebView) {
        let _ = self.event_tx.try_send(RuntimeEvent::Delegate(DelegateEvent::Closed));
    }

    fn request_authentication(&self, _: WebView, request: AuthenticationRequest) {
        let scope = if request.for_proxy() { "proxy" } else { "origin" };
        warn!(
            "authentication requested for {} ({scope}); no prompt implemented, denying",
            request.url()
        );
    }

    fn notify_load_status_changed(&self, _: WebView, _: LoadStatus) {}
}

struct TerminalServoDelegate;

impl ServoDelegate for TerminalServoDelegate {
    fn notify_error(&self, error: ServoError) {
        error!("servo error: {error:?}");
    }
}

// ---------------------------------------------------------------------------
// Logger setup
// ---------------------------------------------------------------------------

fn init_logger(debug: bool) -> io::Result<Option<PathBuf>> {
    if !debug {
        // In release mode, swallow all logs below error level.
        WriteLogger::init(LevelFilter::Error, Config::default(), io::sink())
            .ok();
        return Ok(None);
    }

    let path = std::env::temp_dir()
        .join(format!("carboxyl-{}.log", std::process::id()));
    let file = OpenOptions::new()
        .create(true).truncate(true).write(true)
        .open(&path)?;

    WriteLogger::init(LevelFilter::Debug, Config::default(), file).ok();

    Ok(Some(path))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(cli: Cli) -> AppResult<()> {
    let log_path = init_logger(cli.debug)?;

    ensure_rustls_provider_installed();

    // Bounded channels — bounded prevents unbounded memory growth under load.
    let (event_tx, event_rx) = mpsc::sync_channel::<RuntimeEvent>(512);
    let (servo_tx, servo_rx) = mpsc::sync_channel::<ServoCommand>(128);

    let window = {
        // Read window size before ratatui enters alt screen, so TIOCGWINSZ
        // reflects the real terminal dimensions.
        let terminal = ratatui::init();
        crossterm::execute!(io::stdout(), EnableMouseCapture)?;
        let w = Window::read(&cli);
        (terminal, w)
    };
    let (terminal, window) = window;

    // Spawn the Servo thread — Servo and WebView are created inside so they
    // never need to be Send.
    let servo_handle = {
        let event_tx = event_tx.clone();
        let servo_tx_waker = servo_tx.clone();
        let url = normalize_url(cli.url.clone())?;
        let browser_size = physical_size(window.browser);
        thread::spawn(move || {
            servo_thread(
                event_tx,
                servo_tx_waker,
                servo_rx,
                url,
                browser_size,
            );
        })
    };

    let _input = spawn_input_thread(event_tx.clone());

    let result = event_loop(
        servo_tx.clone(),
        terminal,
        window,
        &cli,
        event_rx,
    );

    let _ = servo_tx.try_send(ServoCommand::Shutdown);
    let _ = servo_handle.join();

    crossterm::execute!(io::stdout(), DisableMouseCapture)?;
    ratatui::restore();

    if let Some(path) = log_path
        && std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
    {
        eprintln!("carboxyl logs written to {}", path.display());
    }

    result
}

// ---------------------------------------------------------------------------
// Servo thread
//
// Owns `servo::Servo`, `WebView`, and the `SoftwareRenderingContext`.
// Receives `ServoCommand`s and sends back frames + delegate events via
// the shared `event_tx`.
// ---------------------------------------------------------------------------

fn servo_thread(
    event_tx: mpsc::SyncSender<RuntimeEvent>,
    servo_tx: mpsc::SyncSender<ServoCommand>,
    servo_rx: mpsc::Receiver<ServoCommand>,
    url: Url,
    browser_size: PhysicalSize<u32>,
) {
    // If Servo's internal threads panic (e.g. StyleThread stack overflow on
    // complex pages), send Exit so the main loop restores the terminal cleanly.
    {
        let tx = event_tx.clone();
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            error!("servo panic: {info}");
            let _ = tx.try_send(RuntimeEvent::Exit);
            default_hook(info);
        }));
    }

    let servo = ServoBuilder::default()
        .preferences(browser_preferences(Preferences::default()))
        .event_loop_waker(Box::new(ServoWaker { tx: servo_tx }))
        .build();

    servo.set_delegate(Rc::new(TerminalServoDelegate));

    let rendering_context: Rc<dyn RenderingContext> = match SoftwareRenderingContext::new(browser_size) {
        Ok(ctx) => Rc::new(ctx),
        Err(e) => {
            error!("failed to create rendering context: {e:?}");
            return;
        }
    };

    let delegate: Rc<dyn WebViewDelegate> = Rc::new(TerminalWebViewDelegate {
        event_tx: event_tx.clone(),
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate)
        .url(url)
        .build();

    webview.show();
    webview.focus();

    // Drain initial wakes so we spin at least once before blocking.
    let _ = event_tx.try_send(RuntimeEvent::Wake);

    while let Ok(cmd) = servo_rx.recv() {

        let mut should_paint = false;
        let mut new_size: Option<PhysicalSize<u32>> = None;

        let mut handle = |cmd: ServoCommand| -> bool {
            match cmd {
                ServoCommand::Shutdown => return false,
                ServoCommand::Load(url) => webview.load(url),
                ServoCommand::GoBack => { if webview.can_go_back() { webview.go_back(1); } }
                ServoCommand::GoForward => { if webview.can_go_forward() { webview.go_forward(1); } }
                ServoCommand::Reload => webview.reload(),
                ServoCommand::Resize(size) => new_size = Some(size),
                ServoCommand::Input(ev) => { webview.notify_input_event(ev); }
                ServoCommand::Paint => should_paint = true,
            }
            true
        };

        if !handle(cmd) { break; }

        // Drain any additional pending commands.
        while let Ok(cmd) = servo_rx.try_recv() {
            if !handle(cmd) { break; }
        }

        if let Some(size) = new_size {
            rendering_context.resize(size);
            webview.resize(size);
            should_paint = true;
        }

        servo.spin_event_loop();

        if should_paint
            && let Some(frame) = paint(&webview, rendering_context.as_ref())
        {
            let _ = event_tx.try_send(RuntimeEvent::Frame(frame));
        }
    }
}

fn paint(webview: &WebView, ctx: &dyn RenderingContext) -> Option<BrowserFrame> {
    ctx.make_current().ok()?;
    webview.paint();

    let image = ctx.read_to_image(DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(
            ctx.size().width as i32,
            ctx.size().height as i32,
        ),
    ))?;

    ctx.present();

    let size = UVec2::new(image.width(), image.height());
    Some(BrowserFrame { pixels: image.into_raw(), size })
}

// ---------------------------------------------------------------------------
// Main event loop — owns all UI state, never touches Servo directly
// ---------------------------------------------------------------------------

fn event_loop(
    servo_tx: mpsc::SyncSender<ServoCommand>,
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

    // fps cap: minimum ms between ratatui draws
    let frame_budget = Duration::from_millis(1000 / cli.fps.max(1) as u64);
    let mut last_draw = std::time::Instant::now() - frame_budget;

    const IDLE_TIMEOUT: Duration = Duration::from_millis(250);

    while running {
        // Detect terminal resize.
        let next = Window::read(cli);
        if next.differs_from(&window) {
            let _ = servo_tx.try_send(ServoCommand::Resize(physical_size(next.browser)));
            window = next;
            pending_paint = true;
        }

        match event_rx.recv_timeout(IDLE_TIMEOUT) {
            Ok(RuntimeEvent::Input(event)) => {
                handle_input(event, &servo_tx, &window, &mut nav, &mut pointer, &mut true_color, &mut running)?;
                while let Ok(RuntimeEvent::Input(event)) = event_rx.try_recv() {
                    handle_input(event, &servo_tx, &window, &mut nav, &mut pointer, &mut true_color, &mut running)?;
                }
                pending_paint = true;
            }

            Ok(RuntimeEvent::Wake) => {
                let _ = servo_tx.try_send(ServoCommand::Paint);
            }

            Ok(RuntimeEvent::Frame(f)) => {
                frame = Some(f);
                pending_paint = true;
            }

            Ok(RuntimeEvent::Delegate(ev)) => {
                match ev {
                    DelegateEvent::UrlChanged(url) => {
                        nav.push(&url, nav.can_go_back, nav.can_go_forward);
                    }
                    DelegateEvent::TitleChanged(title) => {
                        let _ = write!(io::stdout(), "\x1b]0;{title}\x07");
                        let _ = io::stdout().flush();
                    }
                    DelegateEvent::HistoryChanged { url, can_go_back, can_go_forward } => {
                        nav.push(&url, can_go_back, can_go_forward);
                    }
                    DelegateEvent::Closed => running = false,
                }
                pending_paint = true;
            }

            Ok(RuntimeEvent::Exit) => running = false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Rate-limit draws to cli.fps.
        if pending_paint && last_draw.elapsed() >= frame_budget {
            pending_paint = false;
            last_draw = std::time::Instant::now();
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
    servo_tx: &mpsc::SyncSender<ServoCommand>,
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
            let ev = InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: 0.0,
                    y: delta as f64 * window.cell_pixels.y as f64,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                pointer.to_webview_point(),
            ));
            let _ = servo_tx.try_send(ServoCommand::Input(ev));
        }

        input::Event::KeyPress(key) => {
            let action = nav.keypress(key.char, key.modifiers.alt, key.modifiers.meta);
            dispatch_nav(action, servo_tx)?;

            if let Some((down, up)) = map_keyboard_event(&key) {
                let _ = servo_tx.try_send(ServoCommand::Input(InputEvent::Keyboard(down)));
                let _ = servo_tx.try_send(ServoCommand::Input(InputEvent::Keyboard(up)));
            }
        }

        input::Event::MouseDown { row, col } => {
            let action = nav.mouse_down(col, row);
            if matches!(action, NavAction::Forward) {
                let p = BrowserPoint::from_cell(window, col, row);
                *pointer = p;
                let ev = InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Down, MouseButton::Left, p.to_webview_point(),
                ));
                let _ = servo_tx.try_send(ServoCommand::Input(ev));
            } else {
                dispatch_nav(action, servo_tx)?;
            }
        }

        input::Event::MouseUp { row, col } => {
            let action = nav.mouse_up(col, row);
            if matches!(action, NavAction::Forward) {
                let p = BrowserPoint::from_cell(window, col, row);
                *pointer = p;
                let ev = InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Up, MouseButton::Left, p.to_webview_point(),
                ));
                let _ = servo_tx.try_send(ServoCommand::Input(ev));
            } else {
                dispatch_nav(action, servo_tx)?;
            }
        }

        input::Event::MouseMove { row, col } => {
            let p = BrowserPoint::from_cell(window, col, row);
            *pointer = p;
            let ev = InputEvent::MouseMove(MouseMoveEvent::new(p.to_webview_point()));
            let _ = servo_tx.try_send(ServoCommand::Input(ev));
        }
    }

    Ok(())
}

fn dispatch_nav(action: NavAction, servo_tx: &mpsc::SyncSender<ServoCommand>) -> AppResult<()> {
    match action {
        NavAction::Ignore | NavAction::Forward => {}
        NavAction::GoBack    => { let _ = servo_tx.try_send(ServoCommand::GoBack); }
        NavAction::GoForward => { let _ = servo_tx.try_send(ServoCommand::GoForward); }
        NavAction::Refresh   => { let _ = servo_tx.try_send(ServoCommand::Reload); }
        NavAction::GoTo(url) => {
            let _ = servo_tx.try_send(ServoCommand::Load(normalize_url(Some(url))?));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn draw_frame(
    terminal: &mut DefaultTerminal,
    nav: &NavState,
    frame: Option<&BrowserFrame>,
    true_color: bool,
) -> AppResult<()> {
    terminal.draw(|f: &mut Frame| {
        let [nav_area, browser_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(f.area());

        f.render_widget(NavWidget::new(nav), nav_area);

        if let Some(frame) = frame {
            f.render_widget(BrowserWidget::new(frame, true_color), browser_area);
        }

        if let Some(pos) = NavWidget::new(nav).cursor_position(nav_area) {
            f.set_cursor_position(pos);
        }
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Input thread — pure crossterm translator, never touches Servo or state
// ---------------------------------------------------------------------------

fn spawn_input_thread(tx: mpsc::SyncSender<RuntimeEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            match ct_poll(Duration::from_millis(100)) {
                Err(e) => { error!("crossterm poll: {e}"); break; }
                Ok(false) => {
                    // Channel liveness check.
                    if tx.try_send(RuntimeEvent::Wake).is_err() { break; }
                    continue;
                }
                Ok(true) => {}
            }

            match ct_read() {
                Err(e) => { error!("crossterm read: {e}"); break; }
                Ok(ev) => {
                    for event in map_crossterm_event(ev) {
                        let is_exit = matches!(event, input::Event::Exit);
                        let _ = tx.try_send(RuntimeEvent::Input(event));
                        if is_exit { return; }
                    }
                }
            }
        }

        let _ = tx.try_send(RuntimeEvent::Exit);
    })
}

// ---------------------------------------------------------------------------
// Servo configuration
// ---------------------------------------------------------------------------

fn browser_preferences(mut p: Preferences) -> Preferences {
    p.network_http_proxy_uri.clear();
    p.network_https_proxy_uri.clear();
    p.network_http_no_proxy.clear();
    p
}

fn ensure_rustls_provider_installed() {
    if CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}

fn normalize_url(raw: Option<String>) -> AppResult<Url> {
    let Some(raw) = raw else { return Ok(Url::parse("about:blank")?); };

    if raw.contains("://") || raw.starts_with("about:") {
        return Ok(Url::parse(&raw)?);
    }

    if Path::new(&raw).exists() {
        return Url::from_file_path(Path::new(&raw)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("failed to convert path to file URL: {raw}"),
            ).into()
        });
    }

    Ok(Url::parse(&format!("https://{raw}"))?)
}

// ---------------------------------------------------------------------------
// Keyboard mapping
// ---------------------------------------------------------------------------

fn map_keyboard_event(key: &Key) -> Option<(ServoKeyboardEvent, ServoKeyboardEvent)> {
    let (logical_key, code, mut modifiers) = map_logical_key(key)?;
    modifiers |= modifiers_from_key(key);

    let make = |state| ServoKeyboardEvent::new_without_event(
        state, logical_key.clone(), code, Location::Standard, modifiers, false, false,
    );

    Some((make(KeyState::Down), make(KeyState::Up)))
}

fn map_logical_key(key: &Key) -> Option<(ServoKey, Code, ServoModifiers)> {
    let (char_code, forced_ctrl) = match key.char {
        0x01..=0x1a => (key.char + b'a' - 1, true),
        v => (v, false),
    };

    let modifiers = if forced_ctrl { ServoModifiers::CONTROL } else { ServoModifiers::empty() };

    match char_code {
        0x09 => Some((ServoKey::Named(NamedKey::Tab),       Code::Tab,        modifiers)),
        0x0a | 0x0d => Some((ServoKey::Named(NamedKey::Enter),  Code::Enter,  modifiers)),
        0x11 => Some((ServoKey::Named(NamedKey::ArrowUp),    Code::ArrowUp,   modifiers)),
        0x12 => Some((ServoKey::Named(NamedKey::ArrowDown),  Code::ArrowDown, modifiers)),
        0x13 => Some((ServoKey::Named(NamedKey::ArrowRight), Code::ArrowRight,modifiers)),
        0x14 => Some((ServoKey::Named(NamedKey::ArrowLeft),  Code::ArrowLeft, modifiers)),
        0x1b => Some((ServoKey::Named(NamedKey::Escape),     Code::Escape,    modifiers)),
        0x20 => Some((ServoKey::Character(" ".into()),       Code::Space,     modifiers)),
        0x7f => Some((ServoKey::Named(NamedKey::Backspace),  Code::Backspace, modifiers)),
        v if v.is_ascii() => {
            let ch = v as char;
            Some((ServoKey::Character(ch.to_string()), character_code(ch)?, modifiers))
        }
        _ => None,
    }
}

fn character_code(ch: char) -> Option<Code> {
    Some(match ch.to_ascii_lowercase() {
        'a'  => Code::KeyA,  'b'  => Code::KeyB,  'c'  => Code::KeyC,
        'd'  => Code::KeyD,  'e'  => Code::KeyE,  'f'  => Code::KeyF,
        'g'  => Code::KeyG,  'h'  => Code::KeyH,  'i'  => Code::KeyI,
        'j'  => Code::KeyJ,  'k'  => Code::KeyK,  'l'  => Code::KeyL,
        'm'  => Code::KeyM,  'n'  => Code::KeyN,  'o'  => Code::KeyO,
        'p'  => Code::KeyP,  'q'  => Code::KeyQ,  'r'  => Code::KeyR,
        's'  => Code::KeyS,  't'  => Code::KeyT,  'u'  => Code::KeyU,
        'v'  => Code::KeyV,  'w'  => Code::KeyW,  'x'  => Code::KeyX,
        'y'  => Code::KeyY,  'z'  => Code::KeyZ,
        '0'  => Code::Digit0,'1'  => Code::Digit1,'2'  => Code::Digit2,
        '3'  => Code::Digit3,'4'  => Code::Digit4,'5'  => Code::Digit5,
        '6'  => Code::Digit6,'7'  => Code::Digit7,'8'  => Code::Digit8,
        '9'  => Code::Digit9,
        '-'  => Code::Minus,       '='  => Code::Equal,
        '['  => Code::BracketLeft, ']'  => Code::BracketRight,
        '\\' => Code::Backslash,   ';'  => Code::Semicolon,
        '\'' => Code::Quote,       ','  => Code::Comma,
        '.'  => Code::Period,      '/'  => Code::Slash,
        '`'  => Code::Backquote,
        _ => return None,
    })
}

fn modifiers_from_key(key: &Key) -> ServoModifiers {
    let mut m = ServoModifiers::empty();
    if key.modifiers.alt   { m |= ServoModifiers::ALT;     }
    if key.modifiers.ctrl  { m |= ServoModifiers::CONTROL; }
    if key.modifiers.meta  { m |= ServoModifiers::META;    }
    if key.modifiers.shift { m |= ServoModifiers::SHIFT;   }
    m
}
