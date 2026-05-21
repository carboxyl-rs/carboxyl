use std::sync::mpsc;
use std::thread;

use log::{error, warn};
use signal_hook::consts::{SIGINT, SIGPIPE, SIGTERM};
use signal_hook::iterator::Signals;

use super::events::RuntimeEvent;

pub fn spawn_signal_thread(tx: mpsc::SyncSender<RuntimeEvent>) -> thread::JoinHandle<()> {
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
