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

/// Returns true if this crossterm event is a mouse event. Used to detect
/// the SGR 'm'/'M' terminator leak bug (see `is_sgr_artifact` below).
pub fn is_mouse_event(event: &CrosstermEvent) -> bool {
    matches!(event, CrosstermEvent::Mouse(_))
}

/// In SGR mouse mode, mouse release events are terminated with a lowercase
/// 'm' and press events with uppercase 'M'. If crossterm reads a mouse event
/// and a key event from the same buffer, the 'm'/'M' terminator can leak
/// through as a spurious `KeyCode::Char('m'/'M')` with no modifiers.
///
/// This is a known crossterm bug. We suppress the artifact when it
/// immediately follows a mouse event in the same read cycle.
pub fn is_sgr_artifact(event: &CrosstermEvent) -> bool {
    matches!(
        event,
        CrosstermEvent::Key(KeyEvent {
            code: KeyCode::Char('m') | KeyCode::Char('M'),
            modifiers: KeyModifiers::NONE,
            ..
        })
    )
}

/// Translate a crossterm `KeyEvent` into our `Key`, returning `None` for
/// keys we don't handle (function keys, media keys, etc.).
pub fn map_key_event(event: KeyEvent) -> Option<Key> {
    let mods = KeyMods::from_crossterm(event.modifiers);

    let char = match event.code {
        KeyCode::Char(c) if event.modifiers.contains(KeyModifiers::CONTROL) => {
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
pub fn map_crossterm_event(event: CrosstermEvent) -> Vec<Event> {
    match event {
        CrosstermEvent::Key(key_event) => {
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

        CrosstermEvent::Resize(..) => vec![],
        _ => vec![],
    }
}
