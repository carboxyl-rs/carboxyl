// src/browser/servo/session.rs

mod app_state;
mod dispatch;
mod render;
mod timing;

use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Duration;

use color_eyre::eyre::{Result, WrapErr};
use ratatui::DefaultTerminal;

use super::BrowserConfig;
use super::events::{RuntimeEvent, ServoCommand};
use super::geometry::physical_size;

pub use app_state::AppState;
pub use timing::{RenderConfig, TimingState};

// How long to block waiting for events before doing an idle tick.
// Sets the floor for repaint latency when the event stream goes quiet.
const IDLE_TIMEOUT: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

pub struct Channels {
    pub servo_tx: mpsc::SyncSender<ServoCommand>,
    pub event_rx: mpsc::Receiver<RuntimeEvent>,
    pub terminal: DefaultTerminal,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn run(channels: Channels, cfg: &BrowserConfig) -> Result<()> {
    Session::new(channels, cfg).run()
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

struct Session {
    app: AppState,
    cfg: RenderConfig,
    timing: TimingState,
    servo_tx: mpsc::SyncSender<ServoCommand>,
    terminal: DefaultTerminal,
    event_rx: mpsc::Receiver<RuntimeEvent>,
}

impl Session {
    fn new(channels: Channels, browser_cfg: &BrowserConfig) -> Self {
        let cfg = RenderConfig::new(
            browser_cfg.true_color,
            browser_cfg.native_text,
            browser_cfg.fps,
        );
        Self {
            app: AppState::new(browser_cfg.window.clone()),
            timing: TimingState::new(cfg.frame_budget),
            cfg,
            servo_tx: channels.servo_tx,
            terminal: channels.terminal,
            event_rx: channels.event_rx,
        }
    }

    fn run(&mut self) -> Result<()> {
        while self.app.running {
            match self.event_rx.recv_timeout(IDLE_TIMEOUT) {
                Ok(ev) => self.handle_event(ev)?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    log::error!("event channel disconnected — servo thread may have crashed");
                    break;
                }
            }

            self.maybe_paint()?;
        }

        Ok(())
    }

    fn handle_event(&mut self, ev: RuntimeEvent) -> Result<()> {
        match ev {
            RuntimeEvent::Input(event) => {
                let is_scroll = dispatch::handle_input(event, &self.servo_tx, &mut self.app)
                    .wrap_err("failed to handle input event")?;
                let batch_scroll =
                    dispatch::drain_pending_inputs(&self.event_rx, &self.servo_tx, &mut self.app)
                        .wrap_err("failed to drain pending input events")?;

                if (is_scroll || batch_scroll) && self.cfg.native_text {
                    self.schedule_extract();
                }

                self.app.mark_dirty();
            }

            RuntimeEvent::Wake => {
                if self.timing.paint_cmd_due(self.cfg.frame_budget) {
                    if self.servo_tx.try_send(ServoCommand::Paint).is_err() {
                        log::trace!("paint command dropped — servo channel at capacity");
                    }
                    self.timing.mark_paint_cmd();
                }
                if self.cfg.native_text && self.timing.extract_due() {
                    self.schedule_extract();
                }
            }

            RuntimeEvent::Resize(cols, rows) => {
                if let Some(new_window) = self.app.apply_resize(cols, rows)
                    && self
                        .servo_tx
                        .try_send(ServoCommand::Resize(physical_size(new_window.browser)))
                        .is_err()
                {
                    log::trace!("resize command dropped — servo channel at capacity");
                }
                if self.cfg.native_text && self.timing.extract_due() {
                    self.schedule_extract();
                }
            }

            RuntimeEvent::Frame(f) => {
                self.app.apply_frame(f);
            }

            RuntimeEvent::Delegate(ev) => {
                if let Some(title) = self.app.apply_delegate(ev) {
                    let _ = write!(io::stdout(), "\x1b]0;{title}\x07");
                    let _ = io::stdout().flush();
                }
                self.timing.invalidate_extract();
            }

            RuntimeEvent::TextNodes(nodes) => {
                self.app.apply_text_nodes(nodes, self.cfg.native_text);
            }

            RuntimeEvent::TextExtractRequested => {
                if self.cfg.native_text {
                    self.schedule_extract();
                }
            }

            RuntimeEvent::Exit => self.app.stop(),
        }

        Ok(())
    }

    fn maybe_paint(&mut self) -> Result<()> {
        if self.app.pending_paint && self.timing.draw_due(self.cfg.frame_budget) {
            self.app.pending_paint = false;
            self.timing.mark_drawn();
            render::draw_frame(&mut self.terminal, &self.app, &self.cfg)?;
        }
        Ok(())
    }

    fn schedule_extract(&mut self) {
        if self.servo_tx.try_send(ServoCommand::ExtractText).is_err() {
            log::trace!("extract command dropped — servo channel at capacity");
        }
        self.timing.mark_extracted();
    }
}
