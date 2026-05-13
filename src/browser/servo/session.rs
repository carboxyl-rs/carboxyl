mod app_state;
mod dispatch;
mod render;
mod timing;

use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Duration;

use color_eyre::eyre::Result;
use ratatui::DefaultTerminal;

use super::BrowserConfig;
use super::events::{RuntimeEvent, ServoCommand};
use super::geometry::physical_size;

pub use app_state::AppState;
pub use timing::{RenderConfig, TimingState};

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
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            self.maybe_paint()?;
        }

        Ok(())
    }

    fn handle_event(&mut self, ev: RuntimeEvent) -> Result<()> {
        match ev {
            RuntimeEvent::Input(event) => {
                let is_scroll = dispatch::handle_input(event, &self.servo_tx, &mut self.app)?;
                let batch_scroll =
                    dispatch::drain_pending_inputs(&self.event_rx, &self.servo_tx, &mut self.app)?;

                if (is_scroll || batch_scroll) && self.cfg.native_text {
                    self.request_extract();
                }

                self.app.mark_dirty();
            }

            RuntimeEvent::Wake => {
                if self.timing.paint_cmd_due(self.cfg.frame_budget) {
                    let _ = self.servo_tx.try_send(ServoCommand::Paint);
                    self.timing.mark_paint_cmd();
                }
                if self.cfg.native_text && self.timing.extract_due() {
                    self.request_extract();
                }
            }

            RuntimeEvent::Resize(cols, rows) => {
                if let Some(new_window) = self.app.apply_resize(cols, rows) {
                    let _ = self
                        .servo_tx
                        .try_send(ServoCommand::Resize(physical_size(new_window.browser)));
                }
                if self.cfg.native_text && self.timing.extract_due() {
                    self.request_extract();
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
                    self.request_extract();
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

    fn request_extract(&mut self) {
        let _ = self.servo_tx.try_send(ServoCommand::ExtractText);
        self.timing.mark_extracted();
    }
}
