use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyModifiers, MouseButton as CrosstermMouseButton,
    MouseEvent, MouseEventKind,
};

use servo::{KeyboardEvent as ServoKeyboardEvent, Modifiers as ServoModifiers};

use crate::browser::servo::map_keyboard_event;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButtonState {
    Down,
    Up,
}

#[derive(Clone, Debug)]
pub enum Event {
    Keyboard(ServoKeyboardEvent),

    MouseButton {
        button: MouseButton,
        state: MouseButtonState,
        row: u16,
        col: u16,
        modifiers: ServoModifiers,
    },

    MouseMove {
        row: u16,
        col: u16,
        modifiers: ServoModifiers,
    },

    Scroll {
        delta_x: isize,
        delta_y: isize,
        row: u16,
        col: u16,
        modifiers: ServoModifiers,
    },

    Exit,
}

impl Event {
    pub fn from_crossterm(event: CrosstermEvent) -> Vec<Self> {
        match event {
            CrosstermEvent::Key(key_event) => {
                if key_event.code == KeyCode::Char('c')
                    && key_event.modifiers.contains(KeyModifiers::CONTROL)
                {
                    return vec![Event::Exit];
                }

                map_keyboard_event(key_event)
                    .map(Event::Keyboard)
                    .into_iter()
                    .collect()
            }

            CrosstermEvent::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers,
            }) => {
                let modifiers = map_mouse_modifiers(modifiers);

                match kind {
                    MouseEventKind::Down(button) => map_mouse_button(button)
                        .map(|button| Event::MouseButton {
                            button,
                            state: MouseButtonState::Down,
                            row,
                            col: column,
                            modifiers,
                        })
                        .into_iter()
                        .collect(),

                    MouseEventKind::Up(button) => map_mouse_button(button)
                        .map(|button| Event::MouseButton {
                            button,
                            state: MouseButtonState::Up,
                            row,
                            col: column,
                            modifiers,
                        })
                        .into_iter()
                        .collect(),

                    MouseEventKind::Drag(_) => {
                        vec![Event::MouseMove {
                            row,
                            col: column,
                            modifiers,
                        }]
                    }

                    MouseEventKind::Moved => {
                        vec![Event::MouseMove {
                            row,
                            col: column,
                            modifiers,
                        }]
                    }

                    MouseEventKind::ScrollUp => {
                        vec![Event::Scroll {
                            delta_x: 0,
                            delta_y: 1,
                            row,
                            col: column,
                            modifiers,
                        }]
                    }

                    MouseEventKind::ScrollDown => {
                        vec![Event::Scroll {
                            delta_x: 0,
                            delta_y: -1,
                            row,
                            col: column,
                            modifiers,
                        }]
                    }

                    MouseEventKind::ScrollLeft => {
                        vec![Event::Scroll {
                            delta_x: -1,
                            delta_y: 0,
                            row,
                            col: column,
                            modifiers,
                        }]
                    }

                    MouseEventKind::ScrollRight => {
                        vec![Event::Scroll {
                            delta_x: 1,
                            delta_y: 0,
                            row,
                            col: column,
                            modifiers,
                        }]
                    }
                }
            }

            CrosstermEvent::Resize(..) => vec![],

            CrosstermEvent::FocusGained => vec![],
            CrosstermEvent::FocusLost => vec![],
            CrosstermEvent::Paste(_) => vec![],
        }
    }
}

fn map_mouse_button(button: CrosstermMouseButton) -> Option<MouseButton> {
    Some(match button {
        CrosstermMouseButton::Left => MouseButton::Left,
        CrosstermMouseButton::Middle => MouseButton::Middle,
        CrosstermMouseButton::Right => MouseButton::Right,
    })
}

fn map_mouse_modifiers(mods: KeyModifiers) -> ServoModifiers {
    let mut out = ServoModifiers::empty();

    if mods.contains(KeyModifiers::SHIFT) {
        out |= ServoModifiers::SHIFT;
    }

    if mods.contains(KeyModifiers::CONTROL) {
        out |= ServoModifiers::CONTROL;
    }

    if mods.contains(KeyModifiers::ALT) {
        out |= ServoModifiers::ALT;
    }

    if mods.contains(KeyModifiers::SUPER) {
        out |= ServoModifiers::META;
    }

    out
}
