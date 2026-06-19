use std::sync::mpsc;

use log::warn;
use servo::{
    InputEvent, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, WheelDelta,
    WheelEvent, WheelMode,
};

use crate::input;
use crate::output::NavAction;

use super::app_state::AppState;
use super::events::{RuntimeEvent, ServoCommand};
use super::geometry::BrowserPoint;
use super::url::normalize_url;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Dispatch a single input event, mutating `app` and enqueuing servo commands.
/// Returns `true` if the event was a scroll.
pub fn handle_input(
    event: input::Event,
    servo_tx: &mpsc::SyncSender<ServoCommand>,
    app: &mut AppState,
) -> bool {
    let is_scroll = matches!(event, input::Event::Scroll { .. });

    match event {
        input::Event::Exit => {
            app.stop();
        }

        input::Event::Keyboard(key_event) => {
            let action = app
                .nav
                .keyboard(&key_event.event.key, key_event.event.modifiers);

            let forward = matches!(action, NavAction::Forward);
            dispatch_nav(action, servo_tx);

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
            let p = BrowserPoint::from_cell(&app.window, col, row);
            app.pointer = p;

            let ev = InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: delta_x as f64 * app.window.cell_pixels.x as f64,
                    y: delta_y as f64 * app.window.cell_pixels.y as f64,
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

            let p = BrowserPoint::from_cell(&app.window, col, row);
            app.pointer = p;

            let nav_action = match state {
                input::MouseButtonState::Down => app.nav.mouse_down(col, row),
                input::MouseButtonState::Up => app.nav.mouse_up(col, row),
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
                dispatch_nav(nav_action, servo_tx);
            }
        }

        input::Event::MouseMove { row, col, .. } => {
            let p = BrowserPoint::from_cell(&app.window, col, row);
            app.pointer = p;

            let ev = InputEvent::MouseMove(MouseMoveEvent::new(p.to_webview_point()));
            let _ = servo_tx.try_send(ServoCommand::Input(ev));
        }
    }

    is_scroll
}

/// Drain all immediately-available input events from the channel, dispatching
/// each one. Returns `true` if any of them was a scroll event.
///
/// Only `RuntimeEvent::Input` events are consumed; any other event type left
/// in the channel is left for the main loop to process on the next iteration.
pub fn drain_pending_inputs(
    event_rx: &mpsc::Receiver<RuntimeEvent>,
    servo_tx: &mpsc::SyncSender<ServoCommand>,
    app: &mut AppState,
) -> bool {
    let mut any_scroll = false;

    while let Ok(RuntimeEvent::Input(event)) = event_rx.try_recv() {
        any_scroll |= handle_input(event, servo_tx, app);
    }

    any_scroll
}

// ---------------------------------------------------------------------------
// Private
// ---------------------------------------------------------------------------

/// Translate a navigation action into Servo commands or a URL load.
/// URL parse errors are logged as warnings and otherwise ignored - there is
/// no meaningful way to surface them to the user from this call site.
fn dispatch_nav(action: NavAction, servo_tx: &mpsc::SyncSender<ServoCommand>) {
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
}
