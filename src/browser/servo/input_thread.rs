use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use log::error;
use ratatui::crossterm::event::{Event as CrosstermEvent, poll as ct_poll, read as ct_read};

use crate::input::{self, Event};

use super::events::RuntimeEvent;

/// How long to block on crossterm's event poll before sending a Wake tick.
/// Short enough to keep the main loop responsive; long enough to avoid
/// busy-spinning when the terminal is idle.
const INPUT_POLL_TIMEOUT: Duration = Duration::from_millis(100);

pub fn spawn_input_thread(tx: mpsc::SyncSender<RuntimeEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            match ct_poll(INPUT_POLL_TIMEOUT) {
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

                Ok(CrosstermEvent::Resize(cols, rows)) => {
                    let _ = tx.try_send(RuntimeEvent::Resize(cols, rows));
                }

                Ok(event) => {
                    for event in input::Event::from_crossterm(event) {
                        let is_exit = matches!(event, Event::Exit);

                        if tx.try_send(RuntimeEvent::Input(event)).is_err() {
                            return;
                        }

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
