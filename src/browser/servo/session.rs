mod app_state;
mod dispatch;

use std::io::{self, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Layout};

use crate::output::{Canvas, ComposeOptions, Graphics, NAV_BAR_ROWS, NavWidget, Window};

use super::BrowserConfig;
use super::events::{RuntimeEvent, ServoCommand};
use super::geometry::physical_size;

// Re-exported so the sub-modules can reach these through `super::`.
pub(super) use super::{events, geometry, url};

pub use app_state::AppState;

/// How long to block waiting for events before an idle tick. Sets the floor for
/// repaint latency when the event stream goes quiet.
const IDLE_TIMEOUT: Duration = Duration::from_millis(50);

/// Events drained per turn before painting, so a burst cannot starve the draw.
const EVENT_BATCH: usize = 128;

pub struct Channels {
    pub servo_tx: mpsc::SyncSender<ServoCommand>,
    pub event_rx: mpsc::Receiver<RuntimeEvent>,
    pub terminal: DefaultTerminal,
    pub graphics: Option<Graphics>,
    pub window: Window,
}

pub fn run(channels: Channels, cfg: &BrowserConfig) -> Result<()> {
    Session::new(channels, cfg).run()
}

struct Session {
    app: AppState,
    canvas: Canvas,
    options: ComposeOptions,
    frame_budget: Duration,
    servo_tx: mpsc::SyncSender<ServoCommand>,
    event_rx: mpsc::Receiver<RuntimeEvent>,
    terminal: DefaultTerminal,
    graphics: Option<Graphics>,
    /// Servo has been asked for a frame, or owes us one.
    paint_pending: bool,
    paint_requested_at: Instant,
    drawn_at: Instant,
    resize_pending: Option<dpi::PhysicalSize<u32>>,
}

impl Session {
    fn new(channels: Channels, cfg: &BrowserConfig) -> Self {
        let frame_budget = Duration::from_secs_f64(1.0 / f64::from(cfg.fps.max(1)));
        let past = Instant::now() - frame_budget;
        Self {
            app: AppState::new(channels.window),
            canvas: Canvas::default(),
            options: ComposeOptions {
                true_color: cfg.true_color,
                native_text: cfg.native_text,
            },
            frame_budget,
            servo_tx: channels.servo_tx,
            event_rx: channels.event_rx,
            terminal: channels.terminal,
            graphics: channels.graphics,
            paint_pending: true,
            paint_requested_at: past,
            drawn_at: past,
            resize_pending: None,
        }
    }

    fn run(&mut self) -> Result<()> {
        while self.app.running {
            match self.event_rx.recv_timeout(IDLE_TIMEOUT) {
                Ok(event) => self.handle(event),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    log::error!("event channel disconnected - the servo thread may have crashed");
                    break;
                }
            }

            // Drain every event type in order: a loop that only took input
            // would consume and lose the first frame it met.
            for _ in 0..EVENT_BATCH {
                let Ok(event) = self.event_rx.try_recv() else {
                    break;
                };
                self.handle(event);
            }

            self.tick()?;
        }

        Ok(())
    }

    fn handle(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Input(event) => {
                dispatch::handle_input(event, &self.servo_tx, &mut self.app);
                // The address bar may have changed even if the page has not.
                self.app.mark_dirty();
            }

            RuntimeEvent::Wake => self.paint_pending = true,

            RuntimeEvent::Resize(cols, rows) => {
                if let Some(window) = self.app.apply_resize(cols, rows) {
                    self.resize_pending = Some(physical_size(window.viewport()));
                }
            }

            RuntimeEvent::Frame(frame) => self.app.apply_frame(frame),

            RuntimeEvent::Delegate(event) => {
                if let Some(title) = self.app.apply_delegate(event) {
                    // OSC 0: set the terminal window title.
                    let _ = write!(io::stdout(), "\x1b]0;{title}\x07");
                    let _ = io::stdout().flush();
                }
            }

            RuntimeEvent::Exit => self.app.stop(),
        }
    }

    fn tick(&mut self) -> Result<()> {
        if let Some(size) = self.resize_pending
            && self.servo_tx.try_send(ServoCommand::Resize(size)).is_ok()
        {
            self.resize_pending = None;
        }

        // A wake arriving inside the frame budget is deferred, not dropped: a
        // still page may never send another one after its last frame.
        if self.paint_pending
            && self.paint_requested_at.elapsed() >= self.frame_budget
            && self.servo_tx.try_send(ServoCommand::Paint).is_ok()
        {
            self.paint_pending = false;
            self.paint_requested_at = Instant::now();
        }

        if self.app.dirty && self.drawn_at.elapsed() >= self.frame_budget {
            self.app.dirty = false;
            self.drawn_at = Instant::now();
            self.draw()?;
        }

        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        let app = &self.app;
        let canvas = &mut self.canvas;
        let graphics = &mut self.graphics;
        let options = self.options;

        self.terminal.draw(|target| {
            let [nav_area, page_area] =
                Layout::vertical([Constraint::Length(NAV_BAR_ROWS), Constraint::Fill(1)])
                    .areas(target.area());

            let nav = NavWidget::new(&app.nav);
            target.render_widget(nav, nav_area);

            if let Some(frame) = app.frame.as_ref() {
                let tiles = canvas.compose(
                    page_area,
                    target.buffer_mut(),
                    frame,
                    app.window.cell,
                    options,
                );
                if let Some(graphics) = graphics.as_mut() {
                    graphics.render(
                        frame,
                        app.window.cell,
                        tiles,
                        page_area,
                        target.buffer_mut(),
                    );
                }
            }

            if let Some(position) = nav.cursor_position(nav_area) {
                target.set_cursor_position(position);
            }
        })?;

        Ok(())
    }
}
