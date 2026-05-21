use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crossterm::event::{poll as ct_poll, read as ct_read};
use log::error;

use crate::input::{self, Event};

use super::events::RuntimeEvent;

pub fn spawn_input_thread(tx: mpsc::SyncSender<RuntimeEvent>) -> thread::JoinHandle<()> {
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
