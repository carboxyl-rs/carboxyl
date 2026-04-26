use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

/// A normalised input event. This is what `servo_runtime` receives from the
/// input thread — crossterm's raw types never cross that boundary.
#[derive(Clone, Debug)]
pub enum Event {
    KeyPress(Key),
    MouseDown { row: u16, col: u16 },
    MouseUp { row: u16, col: u16 },
    MouseMove { row: u16, col: u16 },
    Scroll { delta: isize },
    TrueColorSupported,
    Exit,
}

/// A normalised key, carrying the logical character and active modifiers.
#[derive(Clone, Debug)]
pub struct Key {
    /// The logical byte value used for Servo's keyboard event mapping.
    /// Uses the same encoding as the old parser: 0x11–0x14 for arrows,
    /// 0x0d for Enter, 0x1b for Escape, 0x7f for Backspace, etc.
    pub char: u8,
    pub modifiers: KeyMods,
}

#[derive(Clone, Debug, Default)]
pub struct KeyMods {
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
}

impl KeyMods {
    fn from_crossterm(mods: KeyModifiers) -> Self {
        Self {
            alt: mods.contains(KeyModifiers::ALT),
            ctrl: mods.contains(KeyModifiers::CONTROL),
            meta: mods.contains(KeyModifiers::SUPER),
            shift: mods.contains(KeyModifiers::SHIFT),
        }
    }
}

/// Translate a crossterm `KeyEvent` into our `Key`, returning `None` for
/// keys we don't handle (function keys, media keys, etc.).
pub fn map_key_event(event: KeyEvent) -> Option<Key> {
    let mods = KeyMods::from_crossterm(event.modifiers);

    let char = match event.code {
        KeyCode::Char(c) if event.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+A–Z → 0x01–0x1a
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_alphabetic() {
                lower as u8 - b'a' + 1
            } else {
                c as u8
            }
        }
        KeyCode::Char(c) if c.is_ascii() => c as u8,
        KeyCode::Enter => 0x0d,
        KeyCode::Tab => 0x09,
        KeyCode::Backspace => 0x7f,
        KeyCode::Esc => 0x1b,
        KeyCode::Up => 0x11,
        KeyCode::Down => 0x12,
        KeyCode::Right => 0x13,
        KeyCode::Left => 0x14,
        KeyCode::Delete => 0x7f,
        _ => return None,
    };

    Some(Key {
        char,
        modifiers: mods,
    })
}

/// Translate a crossterm `Event` into zero or more of our `Event`s.
/// Returns an empty vec for events we don't care about.
pub fn map_crossterm_event(event: CrosstermEvent) -> Vec<Event> {
    match event {
        CrosstermEvent::Key(key_event) => {
            // Ctrl-C → exit
            if key_event.code == KeyCode::Char('c')
                && key_event.modifiers.contains(KeyModifiers::CONTROL)
            {
                return vec![Event::Exit];
            }

            map_key_event(key_event)
                .map(|k| vec![Event::KeyPress(k)])
                .unwrap_or_default()
        }

        CrosstermEvent::Mouse(MouseEvent {
            kind, column, row, ..
        }) => match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                vec![Event::MouseDown { row, col: column }]
            }
            MouseEventKind::Up(MouseButton::Left) => {
                vec![Event::MouseUp { row, col: column }]
            }
            MouseEventKind::Moved => vec![Event::MouseMove { row, col: column }],
            MouseEventKind::ScrollUp => vec![Event::Scroll { delta: 1 }],
            MouseEventKind::ScrollDown => vec![Event::Scroll { delta: -1 }],
            _ => vec![],
        },

        // Terminal resize is handled by the main loop polling crossterm
        // directly; we don't need to forward it as an event here.
        CrosstermEvent::Resize(..) => vec![],

        _ => vec![],
    }
}
