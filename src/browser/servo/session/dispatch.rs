use std::sync::mpsc;

use log::warn;
use servo::{
    InputEvent, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, WheelDelta,
    WheelEvent, WheelMode,
};

use crate::input;
use crate::output::NavAction;

use super::app_state::AppState;
use super::events::ServoCommand;
use super::geometry::viewport_point;
use super::url::normalize_url;

/// Dispatch one input event, mutating `app` and enqueuing Servo commands.
pub fn handle_input(
    event: input::Event,
    servo_tx: &mpsc::SyncSender<ServoCommand>,
    app: &mut AppState,
) {
    match event {
        input::Event::Exit => app.stop(),

        input::Event::Keyboard(key_event) => {
            let action = app
                .nav
                .keyboard(&key_event.event.key, key_event.event.modifiers);

            if matches!(action, NavAction::Forward) {
                let _ = servo_tx.try_send(ServoCommand::Input(InputEvent::Keyboard(key_event)));
            } else {
                dispatch_nav(action, servo_tx);
            }
        }

        input::Event::Scroll {
            delta_x,
            delta_y,
            row,
            col,
            ..
        } => {
            let Some(point) = viewport_point(&app.window, col, row) else {
                return;
            };
            // A wheel notch scrolls by one cell, so the page moves by whole
            // rows and the text that is composed onto them stays aligned.
            let delta = WheelDelta {
                x: f64::from(delta_x as i32) * f64::from(app.window.cell.x),
                y: f64::from(delta_y as i32) * f64::from(app.window.cell.y),
                z: 0.0,
                mode: WheelMode::DeltaPixel,
            };
            let _ = servo_tx.try_send(ServoCommand::Input(InputEvent::Wheel(WheelEvent::new(
                delta, point,
            ))));
        }

        input::Event::MouseButton {
            button,
            state,
            row,
            col,
            ..
        } => {
            let nav_action = match state {
                input::MouseButtonState::Down => app.nav.mouse_down(col, row),
                input::MouseButtonState::Up => app.nav.mouse_up(col, row),
            };

            if !matches!(nav_action, NavAction::Forward) {
                dispatch_nav(nav_action, servo_tx);
                return;
            }

            let Some(point) = viewport_point(&app.window, col, row) else {
                return;
            };
            let event = MouseButtonEvent::new(
                match state {
                    input::MouseButtonState::Down => MouseButtonAction::Down,
                    input::MouseButtonState::Up => MouseButtonAction::Up,
                },
                match button {
                    input::MouseButton::Left => MouseButton::Primary,
                    input::MouseButton::Middle => MouseButton::Auxiliary,
                    input::MouseButton::Right => MouseButton::Secondary,
                },
                point,
            );
            let _ = servo_tx.try_send(ServoCommand::Input(InputEvent::MouseButton(event)));
        }

        input::Event::MouseMove { row, col, .. } => {
            let Some(point) = viewport_point(&app.window, col, row) else {
                return;
            };
            let event = InputEvent::MouseMove(MouseMoveEvent::new(point));
            let _ = servo_tx.try_send(ServoCommand::Input(event));
        }
    }
}

/// Translate a navigation action into Servo commands or a URL load.
///
/// URL parse errors are logged and otherwise ignored: there is nowhere to
/// surface them from this call site.
fn dispatch_nav(action: NavAction, servo_tx: &mpsc::SyncSender<ServoCommand>) {
    let command = match action {
        NavAction::Ignore | NavAction::Forward => return,
        NavAction::GoBack => ServoCommand::GoBack,
        NavAction::GoForward => ServoCommand::GoForward,
        NavAction::Refresh => ServoCommand::Reload,
        NavAction::GoTo(raw) => match normalize_url(Some(raw)) {
            Ok(url) => ServoCommand::Load(url),
            Err(error) => {
                warn!("invalid URL: {error}");
                return;
            }
        },
    };
    let _ = servo_tx.try_send(command);
}
