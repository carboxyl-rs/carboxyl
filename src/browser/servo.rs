mod delegates;
mod event_loop;
mod events;
mod geometry;
mod input_thread;
mod keyboard;
mod servo_thread;
mod signal_thread;
mod url;
mod waker;

use std::io;
use std::sync::mpsc;
use std::thread;

use crossterm::event::{
    EnableMouseCapture, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use rustls::crypto::CryptoProvider;

use crate::cli::Cli;
use crate::output::Window;

pub use event_loop::AppResult;
pub use keyboard::map_keyboard_event;

use self::geometry::physical_size;

use crate::logger;
use crate::output::restore_terminal;

pub fn run(cli: Cli) -> AppResult<()> {
    let log_path = logger::init_logger(&cli)?;

    ensure_rustls_provider_installed();

    let (event_tx, event_rx) = mpsc::sync_channel::<events::RuntimeEvent>(512);
    let (servo_tx, servo_rx) = mpsc::sync_channel::<events::ServoCommand>(128);

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

    let true_color = u32::from(crossterm::style::available_color_count()) >= (1u32 << 24);

    let window = Window::read(&cli);

    let servo_handle = {
        let event_tx = event_tx.clone();
        let servo_tx_waker = servo_tx.clone();
        let initial_url = url::normalize_url(cli.url.clone())?;
        let browser_size = physical_size(window.browser);
        let native_text = !cli.no_native_text;

        thread::Builder::new()
            .name("servo".to_owned())
            .stack_size(64 * 1024 * 1024)
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

    let _signal = signal_thread::spawn_signal_thread(event_tx.clone());
    let _input = input_thread::spawn_input_thread(event_tx.clone());

    let result = event_loop::event_loop(
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

fn shutdown(
    servo_tx: &mpsc::SyncSender<events::ServoCommand>,
    servo_handle: thread::JoinHandle<()>,
    log_path: Option<std::path::PathBuf>,
) {
    let _ = servo_tx.try_send(events::ServoCommand::Shutdown);

    let _ = servo_handle.join();

    restore_terminal();

    logger::print_log_path_if_nonempty(log_path);
}

fn ensure_rustls_provider_installed() {
    if CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}
