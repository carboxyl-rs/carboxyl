use std::io::{self, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use log::warn;
use ratatui::layout::{Constraint, Layout};
use ratatui::{DefaultTerminal, Frame};
use servo::{
    InputEvent, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, WheelDelta,
    WheelEvent, WheelMode,
};

use crate::cli::Cli;
use crate::input;
use crate::output::{
    BrowserFrame, BrowserWidget, NavAction, NavState, NavWidget, TextNode, TextOverlay, Window,
};

use super::events::{DelegateEvent, RuntimeEvent, ServoCommand};
use super::geometry::{BrowserPoint, physical_size};
use super::url::normalize_url;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn event_loop(
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
                let is_scroll = matches!(event, input::Event::Scroll { .. });
                handle_input(
                    event,
                    &servo_tx,
                    &window,
                    &mut nav,
                    &mut pointer,
                    &mut running,
                )?;

                let mut any_scroll = is_scroll;
                while let Ok(RuntimeEvent::Input(event)) = event_rx.try_recv() {
                    any_scroll |= matches!(event, input::Event::Scroll { .. });
                    handle_input(
                        event,
                        &servo_tx,
                        &window,
                        &mut nav,
                        &mut pointer,
                        &mut running,
                    )?;
                }

                if any_scroll && native_text {
                    let _ = servo_tx.try_send(ServoCommand::ExtractText);
                    last_extract = Instant::now();
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
                        nav.push(
                            url,
                            crate::output::NavigationCapability {
                                back: nav.nav.back,
                                forward: nav.nav.forward,
                            },
                        );
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
                        nav.push(
                            url,
                            crate::output::NavigationCapability {
                                back: can_go_back,
                                forward: can_go_forward,
                            },
                        );
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
        input::Event::Exit => {
            *running = false;
        }

        input::Event::Keyboard(key_event) => {
            let action = nav.keyboard(&key_event.event.key, key_event.event.modifiers);

            let forward = matches!(action, NavAction::Forward);

            dispatch_nav(action, servo_tx)?;

            if forward {
                let _ = servo_tx.try_send(ServoCommand::Input(InputEvent::Keyboard(key_event)));
            }
        }

        input::Event::Scroll {
            delta_x,
            delta_y,
            row,
            col,
            ..
        } => {
            let p = BrowserPoint::from_cell(window, col, row);

            *pointer = p;

            let ev = InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: delta_x as f64 * window.cell_pixels.x as f64,
                    y: delta_y as f64 * window.cell_pixels.y as f64,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                p.to_webview_point(),
            ));

            let _ = servo_tx.try_send(ServoCommand::Input(ev));
        }

        input::Event::MouseButton {
            button,
            state,
            row,
            col,
            ..
        } => {
            let servo_button = match button {
                input::MouseButton::Left => MouseButton::Left,
                input::MouseButton::Middle => MouseButton::Middle,
                input::MouseButton::Right => MouseButton::Right,
            };

            let p = BrowserPoint::from_cell(window, col, row);

            *pointer = p;

            let nav_action = match state {
                input::MouseButtonState::Down => nav.mouse_down(col, row),

                input::MouseButtonState::Up => nav.mouse_up(col, row),
            };

            if matches!(nav_action, NavAction::Forward) {
                let servo_state = match state {
                    input::MouseButtonState::Down => MouseButtonAction::Down,

                    input::MouseButtonState::Up => MouseButtonAction::Up,
                };

                let ev = InputEvent::MouseButton(MouseButtonEvent::new(
                    servo_state,
                    servo_button,
                    p.to_webview_point(),
                ));

                let _ = servo_tx.try_send(ServoCommand::Input(ev));
            } else {
                dispatch_nav(nav_action, servo_tx)?;
            }
        }

        input::Event::MouseMove { row, col, .. } => {
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
        NavAction::GoTo(raw) => match normalize_url(Some(raw)) {
            Ok(url) => {
                let _ = servo_tx.try_send(ServoCommand::Load(url));
            }
            Err(e) => warn!("invalid URL: {e}"),
        },
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
