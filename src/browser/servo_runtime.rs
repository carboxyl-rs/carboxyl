use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, poll as ct_poll, read as ct_read};
use dpi::PhysicalSize;
use glam::{UVec2, Vec2};
use log::{error, warn};
use ratatui::layout::{Constraint, Layout};
use ratatui::{DefaultTerminal, Frame};
use rustls::crypto::CryptoProvider;
use servo::{
    AuthenticationRequest, Code, DeviceIntPoint, DeviceIntRect, DeviceIntSize, DevicePoint,
    EventLoopWaker, InputEvent, JavaScriptEvaluationError, Key as ServoKey, KeyState,
    KeyboardEvent as ServoKeyboardEvent, LoadStatus, Location, Modifiers as ServoModifiers,
    MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, NamedKey, Preferences,
    RenderingContext, ServoBuilder, ServoDelegate, ServoError, SoftwareRenderingContext, WebView,
    WebViewBuilder, WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
};
use signal_hook::consts::{SIGINT, SIGPIPE, SIGTERM};
use signal_hook::iterator::Signals;
use simplelog::{Config, LevelFilter, WriteLogger};
use url::Url;

use crate::cli::Cli;
use crate::input::{self, Key, map_crossterm_event};
use crate::output::{
    BrowserFrame, BrowserWidget, EXTRACTION_SCRIPT, NavAction, NavState, NavWidget,
    SUPPRESS_TEXT_SCRIPT, TextNode, TextOverlay, Window, parse_js_nodes,
};

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// ---------------------------------------------------------------------------
// Geometry
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

enum RuntimeEvent {
    Input(input::Event),
    /// Servo needs the loop to consider painting.
    Wake,
    /// A fully composited frame from the Servo thread.
    Frame(BrowserFrame),
    Delegate(DelegateEvent),
    /// Terminal was resized to (cols, rows).
    Resize(u16, u16),
    /// Text nodes extracted from the page via JS.
    TextNodes(Vec<TextNode>),
    /// Fired by the delegate after load-complete; causes an immediate extract
    /// (bypassing the debounce) so native text appears as soon as the page settles.
    TextExtractRequested,
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

/// Commands sent from the main loop to the Servo thread.
enum ServoCommand {
    Load(Url),
    GoBack,
    GoForward,
    Reload,
    Resize(PhysicalSize<u32>),
    Input(InputEvent),
    /// Composite and send back a frame if anything changed.
    Paint,
    /// Run the text extraction script and send results back.
    ExtractText,
    /// Inject the text-suppression stylesheet into the current page.
    SuppressText,
    Shutdown,
}

// ---------------------------------------------------------------------------
// Servo EventLoopWaker — wakes the Servo thread via ServoCommand::Paint
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
        let _ = self.tx.try_send(ServoCommand::Paint);
    }
}

// ---------------------------------------------------------------------------
// WebView delegate — pure event emitter, owns nothing
// ---------------------------------------------------------------------------

struct TerminalWebViewDelegate {
    event_tx: mpsc::SyncSender<RuntimeEvent>,
    servo_tx: mpsc::SyncSender<ServoCommand>,
}

impl WebViewDelegate for TerminalWebViewDelegate {
    fn notify_url_changed(&self, _: WebView, url: Url) {
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::UrlChanged(
                url.to_string(),
            )));
    }

    fn notify_page_title_changed(&self, _: WebView, title: Option<String>) {
        let title = title.unwrap_or_else(|| "Carboxyl".to_owned());
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::TitleChanged(title)));
    }

    fn notify_new_frame_ready(&self, _: WebView) {
        let _ = self.event_tx.try_send(RuntimeEvent::Wake);
    }

    fn notify_history_changed(&self, _: WebView, entries: Vec<Url>, current: usize) {
        let Some(url) = entries.get(current) else {
            return;
        };
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::HistoryChanged {
                url: url.to_string(),
                can_go_back: current > 0,
                can_go_forward: current + 1 < entries.len(),
            }));
    }

    fn notify_animating_changed(&self, _: WebView, animating: bool) {
        if animating {
            let _ = self.event_tx.try_send(RuntimeEvent::Wake);
        }
    }

    fn notify_closed(&self, _: WebView) {
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::Closed));
    }

    fn request_authentication(&self, _: WebView, request: AuthenticationRequest) {
        let scope = if request.for_proxy() {
            "proxy"
        } else {
            "origin"
        };
        warn!(
            "authentication requested for {} ({scope}); no prompt implemented, denying",
            request.url()
        );
    }

    fn notify_load_status_changed(&self, _: WebView, status: LoadStatus) {
        if matches!(status, LoadStatus::Complete) {
            // Suppress first so Servo repaints with transparent text, then
            // schedule extraction so the overlay is populated once settled.
            // SuppressText enters servo_rx before TextExtractRequested reaches
            // the main loop, so ordering is guaranteed.
            let _ = self.servo_tx.try_send(ServoCommand::SuppressText);
            let _ = self.event_tx.try_send(RuntimeEvent::TextExtractRequested);
        }
    }
}

struct TerminalServoDelegate;

impl ServoDelegate for TerminalServoDelegate {
    fn notify_error(&self, error: ServoError) {
        error!("servo error: {error:?}");
    }
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

fn init_logger(debug: bool) -> io::Result<Option<PathBuf>> {
    if !debug {
        WriteLogger::init(LevelFilter::Error, Config::default(), io::sink()).ok();
        return Ok(None);
    }

    let path = std::env::temp_dir().join(format!("carboxyl-{}.log", std::process::id()));
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)?;

    WriteLogger::init(LevelFilter::Debug, Config::default(), file).ok();
    Ok(Some(path))
}

// ---------------------------------------------------------------------------
// Graceful shutdown
// ---------------------------------------------------------------------------

fn shutdown(
    servo_tx: &mpsc::SyncSender<ServoCommand>,
    servo_handle: thread::JoinHandle<()>,
    log_path: Option<PathBuf>,
) {
    let _ = servo_tx.try_send(ServoCommand::Shutdown);
    let _ = servo_handle.join();

    crossterm::execute!(io::stdout(), DisableMouseCapture).ok();
    ratatui::restore();

    if let Some(path) = log_path
        && std::fs::metadata(&path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    {
        eprintln!("carboxyl logs written to {}", path.display());
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(cli: Cli) -> AppResult<()> {
    let log_path = init_logger(cli.debug)?;

    ensure_rustls_provider_installed();

    let (event_tx, event_rx) = mpsc::sync_channel::<RuntimeEvent>(512);
    let (servo_tx, servo_rx) = mpsc::sync_channel::<ServoCommand>(128);

    let terminal = ratatui::init();
    crossterm::execute!(io::stdout(), EnableMouseCapture)?;

    let true_color = u32::from(crossterm::style::available_color_count()) >= (1u32 << 24);

    let window = Window::read(&cli);

    let servo_handle = {
        let event_tx = event_tx.clone();
        let servo_tx_waker = servo_tx.clone();
        let url = normalize_url(cli.url.clone())?;
        let browser_size = physical_size(window.browser);

        thread::Builder::new()
            .name("servo".to_owned())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                servo_thread(event_tx, servo_tx_waker, servo_rx, url, browser_size);
            })
            .expect("failed to spawn servo thread")
    };

    let _signal = spawn_signal_thread(event_tx.clone());
    let _input = spawn_input_thread(event_tx.clone());

    let result = event_loop(
        servo_tx.clone(),
        terminal,
        window,
        &cli,
        true_color,
        event_rx,
    );

    shutdown(&servo_tx, servo_handle, log_path);

    result
}

// ---------------------------------------------------------------------------
// Servo thread
// ---------------------------------------------------------------------------

fn servo_thread(
    event_tx: mpsc::SyncSender<RuntimeEvent>,
    servo_tx: mpsc::SyncSender<ServoCommand>,
    servo_rx: mpsc::Receiver<ServoCommand>,
    url: Url,
    browser_size: PhysicalSize<u32>,
) {
    let servo = ServoBuilder::default()
        .preferences(browser_preferences(Preferences::default()))
        .event_loop_waker(Box::new(ServoWaker {
            tx: servo_tx.clone(),
        }))
        .build();

    servo.set_delegate(Rc::new(TerminalServoDelegate));

    let rendering_context: Rc<dyn RenderingContext> =
        match SoftwareRenderingContext::new(browser_size) {
            Ok(ctx) => Rc::new(ctx),
            Err(e) => {
                error!("failed to create rendering context: {e:?}");
                let _ = event_tx.try_send(RuntimeEvent::Exit);
                return;
            }
        };

    let delegate: Rc<dyn WebViewDelegate> = Rc::new(TerminalWebViewDelegate {
        event_tx: event_tx.clone(),
        servo_tx: servo_tx.clone(),
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate)
        .url(url)
        .build();

    webview.show();
    webview.focus();

    while let Ok(cmd) = servo_rx.recv() {
        let mut should_paint = false;
        let mut should_extract = false;
        let mut should_suppress = false;
        let mut new_size: Option<PhysicalSize<u32>> = None;
        let mut shutdown = false;

        let mut handle = |cmd: ServoCommand| match cmd {
            ServoCommand::Shutdown => shutdown = true,
            ServoCommand::Load(url) => webview.load(url),
            ServoCommand::GoBack => {
                if webview.can_go_back() {
                    webview.go_back(1);
                }
            }
            ServoCommand::GoForward => {
                if webview.can_go_forward() {
                    webview.go_forward(1);
                }
            }
            ServoCommand::Reload => webview.reload(),
            ServoCommand::Resize(size) => new_size = Some(size),
            ServoCommand::Input(ev) => {
                webview.notify_input_event(ev);
            }
            ServoCommand::Paint => should_paint = true,
            ServoCommand::ExtractText => should_extract = true,
            ServoCommand::SuppressText => should_suppress = true,
        };

        handle(cmd);

        while let Ok(cmd) = servo_rx.try_recv() {
            handle(cmd);
        }

        if shutdown {
            break;
        }

        if let Some(size) = new_size {
            rendering_context.resize(size);
            webview.resize(size);
            should_paint = true;
        }

        servo.spin_event_loop();

        // Suppress first so Servo repaints with transparent text before we
        // extract node positions — guarantees the two are always paired.
        if should_suppress {
            suppress_text(&webview);
        }

        if should_extract {
            extract_text(&webview, event_tx.clone());
        }

        if should_paint && let Some(frame) = paint(&webview, rendering_context.as_ref()) {
            let _ = event_tx.try_send(RuntimeEvent::Frame(frame));
        }

        thread::sleep(Duration::from_millis(1));
    }
}

/// Inject the text-suppression stylesheet into the current page.
fn suppress_text(webview: &WebView) {
    webview.evaluate_javascript(SUPPRESS_TEXT_SCRIPT, |result| {
        if let Err(e) = result
            && !matches!(e, JavaScriptEvaluationError::WebViewNotReady)
        {
            warn!("text suppression failed: {e:?}");
        }
    });
}

/// Evaluate the text extraction script on the current page and forward
/// the results back to the main loop as `RuntimeEvent::TextNodes`.
fn extract_text(webview: &WebView, event_tx: mpsc::SyncSender<RuntimeEvent>) {
    webview.evaluate_javascript(EXTRACTION_SCRIPT, move |result| match result {
        Ok(value) => {
            let nodes = parse_js_nodes(&value);
            if !nodes.is_empty() {
                let _ = event_tx.try_send(RuntimeEvent::TextNodes(nodes));
            }
        }
        Err(e) => {
            if !matches!(e, JavaScriptEvaluationError::WebViewNotReady) {
                warn!("text extraction failed: {e:?}");
            }
        }
    });
}

fn paint(webview: &WebView, ctx: &dyn RenderingContext) -> Option<BrowserFrame> {
    ctx.make_current().ok()?;
    webview.paint();

    let size = ctx.size();
    let rect = DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(size.width as i32, size.height as i32),
    );

    let image = ctx.read_to_image(rect)?;
    ctx.present();

    Some(BrowserFrame {
        size: UVec2::new(image.width(), image.height()),
        pixels: image.into_raw(),
    })
}

// ---------------------------------------------------------------------------
// Main event loop
// ---------------------------------------------------------------------------

fn event_loop(
    servo_tx: mpsc::SyncSender<ServoCommand>,
    mut terminal: DefaultTerminal,
    mut window: Window,
    cli: &Cli,
    true_color: bool,
    event_rx: mpsc::Receiver<RuntimeEvent>,
) -> AppResult<()> {
    let mut pointer = BrowserPoint::default();
    let mut nav = NavState::default();
    let mut frame: Option<BrowserFrame> = None;
    let mut running = true;
    let mut pending_paint = true;
    let native_text = !cli.no_native_text;

    let frame_budget = Duration::from_millis(1000 / cli.fps.max(1) as u64);
    let mut last_draw = Instant::now() - frame_budget;
    let mut last_paint_cmd = Instant::now() - frame_budget;
    let extract_debounce = Duration::from_millis(300);
    let mut last_extract = Instant::now() - extract_debounce;
    let mut text_nodes: Vec<TextNode> = Vec::new();

    const IDLE_TIMEOUT: Duration = Duration::from_millis(50);

    while running {
        match event_rx.recv_timeout(IDLE_TIMEOUT) {
            Ok(RuntimeEvent::Input(event)) => {
                handle_input(
                    event,
                    &servo_tx,
                    &window,
                    &mut nav,
                    &mut pointer,
                    &mut running,
                )?;
                while let Ok(RuntimeEvent::Input(event)) = event_rx.try_recv() {
                    handle_input(
                        event,
                        &servo_tx,
                        &window,
                        &mut nav,
                        &mut pointer,
                        &mut running,
                    )?;
                }
                pending_paint = true;
            }

            Ok(RuntimeEvent::Wake) => {
                if last_paint_cmd.elapsed() >= frame_budget {
                    let _ = servo_tx.try_send(ServoCommand::Paint);
                    last_paint_cmd = Instant::now();
                }
                if native_text && last_extract.elapsed() >= extract_debounce {
                    let _ = servo_tx.try_send(ServoCommand::ExtractText);
                    last_extract = Instant::now();
                }
            }

            Ok(RuntimeEvent::Resize(cols, rows)) => {
                let next = window.resize(cols, rows);
                if next.differs_from(&window) {
                    let _ = servo_tx.try_send(ServoCommand::Resize(physical_size(next.browser)));
                    window = next;
                }
                if native_text && last_extract.elapsed() >= extract_debounce {
                    let _ = servo_tx.try_send(ServoCommand::ExtractText);
                    last_extract = Instant::now();
                }
                pending_paint = true;
            }

            Ok(RuntimeEvent::Frame(f)) => {
                frame = Some(f);
                pending_paint = true;
            }

            Ok(RuntimeEvent::Delegate(ev)) => {
                match ev {
                    DelegateEvent::UrlChanged(url) => {
                        nav.push(&url, nav.can_go_back, nav.can_go_forward);
                        last_extract = Instant::now() - extract_debounce;
                    }
                    DelegateEvent::TitleChanged(title) => {
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
                    DelegateEvent::Closed => running = false,
                }
                pending_paint = true;
            }

            Ok(RuntimeEvent::TextNodes(nodes)) => {
                if native_text {
                    text_nodes = nodes;
                    pending_paint = true;
                }
            }

            // Load-complete fired by the delegate: extract immediately,
            // bypassing the debounce. SuppressText was already enqueued into
            // servo_rx by the delegate before this event arrived here.
            Ok(RuntimeEvent::TextExtractRequested) => {
                if native_text {
                    let _ = servo_tx.try_send(ServoCommand::ExtractText);
                    last_extract = Instant::now();
                }
            }

            Ok(RuntimeEvent::Exit) => running = false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if pending_paint && last_draw.elapsed() >= frame_budget {
            pending_paint = false;
            last_draw = Instant::now();
            draw_frame(
                &mut terminal,
                &nav,
                frame.as_ref(),
                &text_nodes,
                &window,
                true_color,
                native_text,
            )?;
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
    running: &mut bool,
) -> AppResult<()> {
    match event {
        input::Event::Exit => *running = false,

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
            let forward = matches!(action, NavAction::Forward);
            dispatch_nav(action, servo_tx)?;

            if forward && let Some((down, up)) = map_keyboard_event(&key) {
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
                    MouseButtonAction::Down,
                    MouseButton::Left,
                    p.to_webview_point(),
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
                    MouseButtonAction::Up,
                    MouseButton::Left,
                    p.to_webview_point(),
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
        NavAction::GoBack => {
            let _ = servo_tx.try_send(ServoCommand::GoBack);
        }
        NavAction::GoForward => {
            let _ = servo_tx.try_send(ServoCommand::GoForward);
        }
        NavAction::Refresh => {
            let _ = servo_tx.try_send(ServoCommand::Reload);
        }
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
    text_nodes: &[TextNode],
    window: &Window,
    true_color: bool,
    native_text: bool,
) -> AppResult<()> {
    terminal.draw(|f: &mut Frame| {
        let [nav_area, browser_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(f.area());

        f.render_widget(NavWidget::new(nav), nav_area);

        if let Some(frame) = frame {
            f.render_widget(BrowserWidget::new(frame, true_color), browser_area);

            if native_text && !text_nodes.is_empty() {
                let pixels = Some((frame.pixels.as_slice(), frame.size.x, frame.size.y));
                f.render_widget(
                    TextOverlay::new(text_nodes, window.cell_pixels, pixels, true_color),
                    browser_area,
                );
            }
        }

        if let Some(pos) = NavWidget::new(nav).cursor_position(nav_area) {
            f.set_cursor_position(pos);
        }
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Input thread — pure crossterm translator
// ---------------------------------------------------------------------------

fn spawn_input_thread(tx: mpsc::SyncSender<RuntimeEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            match ct_poll(Duration::from_millis(100)) {
                Err(e) => {
                    error!("crossterm poll: {e}");
                    break;
                }
                Ok(false) => {
                    if tx.try_send(RuntimeEvent::Wake).is_err() {
                        break;
                    }
                    continue;
                }
                Ok(true) => {}
            }

            match ct_read() {
                Err(e) => {
                    error!("crossterm read: {e}");
                    break;
                }
                Ok(crossterm::event::Event::Resize(cols, rows)) => {
                    let _ = tx.try_send(RuntimeEvent::Resize(cols, rows));
                }
                Ok(ev) => {
                    for event in map_crossterm_event(ev) {
                        let is_exit = matches!(event, input::Event::Exit);
                        let _ = tx.try_send(RuntimeEvent::Input(event));
                        if is_exit {
                            return;
                        }
                    }
                }
            }
        }

        let _ = tx.try_send(RuntimeEvent::Exit);
    })
}

// ---------------------------------------------------------------------------
// Signal thread — routes OS signals to RuntimeEvent::Exit
// ---------------------------------------------------------------------------

fn spawn_signal_thread(tx: mpsc::SyncSender<RuntimeEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut signals = match Signals::new([SIGINT, SIGTERM, SIGPIPE]) {
            Ok(s) => s,
            Err(e) => {
                error!("failed to register signal handlers: {e}");
                return;
            }
        };

        if let Some(sig) = signals.forever().next() {
            warn!("received signal {sig}, shutting down");
            let _ = tx.try_send(RuntimeEvent::Exit);
        }
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
                format!("failed to convert path to file URL: {raw}"),
            )
            .into()
        });
    }

    Ok(Url::parse(&format!("https://{raw}"))?)
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
    let empty = ServoModifiers::empty();
    match key.char {
        0x09 => return Some((ServoKey::Named(NamedKey::Tab), Code::Tab, empty)),
        0x0a | 0x0d => return Some((ServoKey::Named(NamedKey::Enter), Code::Enter, empty)),
        0x11 => return Some((ServoKey::Named(NamedKey::ArrowUp), Code::ArrowUp, empty)),
        0x12 => return Some((ServoKey::Named(NamedKey::ArrowDown), Code::ArrowDown, empty)),
        0x13 => {
            return Some((
                ServoKey::Named(NamedKey::ArrowRight),
                Code::ArrowRight,
                empty,
            ));
        }
        0x14 => return Some((ServoKey::Named(NamedKey::ArrowLeft), Code::ArrowLeft, empty)),
        0x1b => return Some((ServoKey::Named(NamedKey::Escape), Code::Escape, empty)),
        0x7f => return Some((ServoKey::Named(NamedKey::Backspace), Code::Backspace, empty)),
        _ => {}
    }

    let (char_code, forced_ctrl) = match key.char {
        0x01..=0x1a => (key.char + b'a' - 1, true),
        v => (v, false),
    };

    let modifiers = if forced_ctrl {
        ServoModifiers::CONTROL
    } else {
        empty
    };

    match char_code {
        0x20 => Some((ServoKey::Character(" ".into()), Code::Space, modifiers)),
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
