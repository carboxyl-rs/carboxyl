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

use color_eyre::eyre::Result;
use crossterm::event::{
    EnableMouseCapture, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use rustls::crypto::CryptoProvider;

use crate::cli::Cli;
use crate::logger;
use crate::output::{Window, restore_terminal};

pub use keyboard::map_keyboard_event;
pub use session::Channels;

use self::geometry::physical_size;

// Servo's layout engine is deeply recursive.
const SERVO_STACK_SIZE: usize = 64 * 1024 * 1024;

// Headroom for bursts without blocking producers.
const EVENT_CHANNEL_CAPACITY: usize = 512;
const COMMAND_CHANNEL_CAPACITY: usize = 128;

// 2^24 distinct colors.
const TRUE_COLOR_COUNT: u32 = 1 << 24;

// ---------------------------------------------------------------------------
// BrowserConfig
// ---------------------------------------------------------------------------

pub struct BrowserConfig {
    pub window: Window,
    pub true_color: bool,
    pub native_text: bool,
    pub fps: u16,
    pub initial_url: ::url::Url,
    pub log_path: Option<PathBuf>,
}

impl BrowserConfig {
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        Ok(Self {
            log_path: logger::init_logger(cli)?,
            window: Window::read(cli),
            true_color: u32::from(crossterm::style::available_color_count()) >= TRUE_COLOR_COUNT,
            native_text: !cli.no_native_text,
            fps: cli.fps,
            initial_url: url::normalize_url(cli.url.clone())?,
        })
    }
}

// ---------------------------------------------------------------------------
// BrowserRuntime
// ---------------------------------------------------------------------------

pub struct BrowserRuntime {
    servo_tx: mpsc::SyncSender<events::ServoCommand>,
    servo_handle: Option<thread::JoinHandle<()>>,
    log_path: Option<PathBuf>,
    _signal: thread::JoinHandle<()>,
    _input: thread::JoinHandle<()>,
}

impl BrowserRuntime {
    pub fn spawn(cfg: &BrowserConfig) -> Result<(Self, Channels)> {
        ensure_rustls_provider_installed();

        let (event_tx, event_rx) =
            mpsc::sync_channel::<events::RuntimeEvent>(EVENT_CHANNEL_CAPACITY);
        let (servo_tx, servo_rx) =
            mpsc::sync_channel::<events::ServoCommand>(COMMAND_CHANNEL_CAPACITY);

        let terminal = ratatui::init();

        crossterm::execute!(
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
            let servo_tx_waker = servo_tx.clone();
            let initial_url = cfg.initial_url.clone();
            let browser_size = physical_size(cfg.window.browser);
            let native_text = cfg.native_text;

            thread::Builder::new()
                .name("servo".to_owned())
                .stack_size(SERVO_STACK_SIZE)
                .spawn(move || {
                    servo_thread::servo_thread(
                        event_tx,
                        servo_tx_waker,
                        servo_rx,
                        initial_url,
                        browser_size,
                        native_text,
                    );
                })
                .expect("failed to spawn servo thread")
        };

        let runtime = Self {
            servo_tx: servo_tx.clone(),
            servo_handle: Some(servo_handle),
            log_path: cfg.log_path.clone(),
            _signal: signal_thread::spawn_signal_thread(event_tx.clone()),
            _input: input_thread::spawn_input_thread(event_tx),
        };

        Ok((
            runtime,
            Channels {
                servo_tx,
                event_rx,
                terminal,
            },
        ))
    }
}

impl Drop for BrowserRuntime {
    fn drop(&mut self) {
        let _ = self.servo_tx.try_send(events::ServoCommand::Shutdown);

        if let Some(handle) = self.servo_handle.take() {
            let _ = handle.join();
        }

        restore_terminal();
        logger::print_log_path_if_nonempty(self.log_path.take());
    }
}

// ---------------------------------------------------------------------------

fn ensure_rustls_provider_installed() {
    if CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}
