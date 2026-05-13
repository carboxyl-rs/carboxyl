use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

// ---------------------------------------------------------------------------
// Logical key representation
// ---------------------------------------------------------------------------

/// The logical meaning of a key press, independent of physical position.
///
/// `Named` variants cover all non-character keys we care about; printable
/// ASCII characters are carried as `Char(char)`. Keys we don't handle produce
/// a `TryFrom` error and are silently dropped at the call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogicalKey {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowRight,
    ArrowLeft,
    Delete,
}

/// A normalised key event: logical meaning plus active modifier flags.
#[derive(Clone, Debug)]
pub struct Key {
    pub logical: LogicalKey,
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

// ---------------------------------------------------------------------------
// TryFrom<KeyEvent> for Key
// ---------------------------------------------------------------------------

/// Fails (returns `Err(())`) for keys we don't handle: function keys, media
/// keys, non-ASCII `Char` events, etc.
impl TryFrom<KeyEvent> for Key {
    type Error = ();

    fn try_from(event: KeyEvent) -> Result<Self, Self::Error> {
        let mods = KeyMods::from_crossterm(event.modifiers);

        let logical = match event.code {
            KeyCode::Enter => LogicalKey::Enter,
            KeyCode::Tab => LogicalKey::Tab,
            KeyCode::Backspace | KeyCode::Delete => LogicalKey::Backspace,
            KeyCode::Esc => LogicalKey::Escape,
            KeyCode::Up => LogicalKey::ArrowUp,
            KeyCode::Down => LogicalKey::ArrowDown,
            KeyCode::Right => LogicalKey::ArrowRight,
            KeyCode::Left => LogicalKey::ArrowLeft,
            KeyCode::Char(c) if c.is_ascii() => LogicalKey::Char(c),
            _ => return Err(()),
        };

        Ok(Key {
            logical,
            modifiers: mods,
        })
    }
}

// ---------------------------------------------------------------------------
// Normalised input event
// ---------------------------------------------------------------------------

/// A normalised input event. Crossterm's raw types never cross this boundary
/// into the browser subsystem.
#[derive(Clone, Debug)]
pub enum Event {
    KeyPress(Key),
    MouseDown { row: u16, col: u16 },
    MouseUp { row: u16, col: u16 },
    MouseMove { row: u16, col: u16 },
    Scroll { delta: isize },
    Exit,
}

impl Event {
    /// Translate a crossterm event into zero or more normalised [`Event`]s.
    ///
    /// Returns a `Vec` because a single crossterm event can legitimately
    /// produce no output (unrecognised key) or exactly one output. A `From`
    /// impl on `Vec<Event>` would violate the orphan rule since both types are
    /// foreign; an associated constructor on our own type is the idiomatic
    /// alternative.
    pub fn from_crossterm(event: CrosstermEvent) -> Vec<Self> {
        match event {
            CrosstermEvent::Key(key_event) => {
                // Ctrl-C is a hard exit regardless of other modifier state.
                if key_event.code == KeyCode::Char('c')
                    && key_event.modifiers.contains(KeyModifiers::CONTROL)
                {
                    return vec![Event::Exit];
                }

                Key::try_from(key_event)
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

            // Resize events are intercepted by the input thread and emitted
            // directly as RuntimeEvent::Resize — they never reach this path.
            CrosstermEvent::Resize(..) => vec![],
            _ => vec![],
        }
    }
}
