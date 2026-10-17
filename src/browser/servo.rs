pub(crate) mod a11y;
mod delegates;
mod events;
mod geometry;
mod input_thread;
mod keyboard;
mod servo_thread;
pub(crate) mod session;
mod signal_thread;
mod url;
mod waker;

use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use anyhow::{Result, anyhow};
use ratatui::crossterm::event::{
    EnableMouseCapture, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::style::available_color_count;
use rustls::crypto::CryptoProvider;

use crate::cli::{Cli, GraphicsMode};
use crate::logger;
use crate::output::{DEFAULT_CELL, Graphics, Window, restore_terminal, terminal_cell_size};

pub use keyboard::map_keyboard_event;
pub use session::Channels;

use self::geometry::physical_size;

// Servo's layout engine is deeply recursive.
const SERVO_STACK_SIZE: usize = 64 * 1024 * 1024;

// Headroom for bursts without blocking producers.
const EVENT_CHANNEL_CAPACITY: usize = 512;
const COMMAND_CHANNEL_CAPACITY: usize = 128;

/// Denominator for the CLI `--scale` percentage.
const SCALE_DIVISOR: f32 = 100.0;

pub struct BrowserConfig {
    pub graphics: GraphicsMode,
    pub native_text: bool,
    pub true_color: bool,
    pub fps: u16,
    /// Servo page zoom; terminal viewport dimensions stay fixed.
    pub zoom: f32,
    pub initial_url: ::url::Url,
    pub log_path: Option<PathBuf>,
}

impl BrowserConfig {
    /// `log_path` must be initialized before this call - use
    /// `logger::init_logger` in `main` and pass the result in.
    pub fn from_cli(cli: &Cli, log_path: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            log_path,
            // crossterm reports true color as `u16::MAX`, the largest count it can express.
            true_color: available_color_count() == u16::MAX,
            graphics: cli.graphics,
            native_text: !cli.no_native_text,
            fps: cli.fps,
            zoom: f32::from(cli.scale.max(1)) / SCALE_DIVISOR,
            initial_url: url::normalize_url(cli.url.clone())?,
        })
    }
}

pub struct BrowserRuntime {
    servo_tx: mpsc::SyncSender<events::ServoCommand>,
    servo_handle: Option<thread::JoinHandle<()>>,
    log_path: Option<PathBuf>,
    stderr_guard: Option<logger::StderrGuard>,
    // Prefixed with `_` because these are held purely for their Drop side-effect
    // (keeping the threads alive). They are never read after construction.
    _signal_handle: thread::JoinHandle<()>,
    _input_handle: thread::JoinHandle<()>,
}

impl BrowserRuntime {
    pub fn spawn(cfg: &BrowserConfig) -> Result<(Self, Channels)> {
        install_rustls_provider()?;
        let stderr_guard = logger::StderrGuard::redirect(cfg.log_path.as_deref())?;

        let (event_tx, event_rx) =
            mpsc::sync_channel::<events::RuntimeEvent>(EVENT_CHANNEL_CAPACITY);
        let (servo_tx, servo_rx) =
            mpsc::sync_channel::<events::ServoCommand>(COMMAND_CHANNEL_CAPACITY);

        let terminal = ratatui::init();

        // Query graphics after entering the alternate screen, before input starts.
        let graphics = Graphics::detect(cfg.graphics);

        // Match terminal pixels so graphics tiles need no resampling.
        let cell = graphics
            .as_ref()
            .map(Graphics::cell_size)
            .or_else(terminal_cell_size)
            .unwrap_or(DEFAULT_CELL);
        let window = Window::read(cell);

        execute!(
            io::stdout(),
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            ),
        )?;

        let servo_handle = {
            let event_tx = event_tx.clone();
            let servo_tx = servo_tx.clone();
            let setup = servo_thread::Setup {
                url: cfg.initial_url.clone(),
                viewport: physical_size(window.viewport()),
                zoom: cfg.zoom,
                native_text: cfg.native_text,
            };

            thread::Builder::new()
                .name("servo".to_owned())
                .stack_size(SERVO_STACK_SIZE)
                .spawn(move || servo_thread::servo_thread(event_tx, servo_tx, servo_rx, setup))
                .expect("failed to spawn servo thread")
        };

        let runtime = Self {
            servo_tx: servo_tx.clone(),
            servo_handle: Some(servo_handle),
            log_path: cfg.log_path.clone(),
            stderr_guard: Some(stderr_guard),
            _signal_handle: signal_thread::spawn_signal_thread(event_tx.clone()),
            _input_handle: input_thread::spawn_input_thread(event_tx),
        };

        Ok((
            runtime,
            Channels {
                servo_tx,
                event_rx,
                terminal,
                graphics,
                window,
            },
        ))
    }
}

impl Drop for BrowserRuntime {
    fn drop(&mut self) {
        // Blocking send is intentional: we must deliver Shutdown before joining.
        // The channel will drain quickly as Servo processes its queue.
        let _ = self.servo_tx.send(events::ServoCommand::Shutdown);

        if let Some(handle) = self.servo_handle.take() {
            let _ = handle.join();
        }

        restore_terminal();
        self.stderr_guard.take();
        logger::print_log_path_if_nonempty(self.log_path.take());
    }
}

// ---------------------------------------------------------------------------

fn install_rustls_provider() -> Result<()> {
    if CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .map_err(|_| anyhow!("failed to install rustls crypto provider"))?;
    }
    Ok(())
}
