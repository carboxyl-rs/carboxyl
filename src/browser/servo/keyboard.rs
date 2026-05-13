use servo::{
    Code, Key as ServoKey, KeyState, KeyboardEvent as ServoKeyboardEvent, Location,
    Modifiers as ServoModifiers, NamedKey,
};

use crate::input::{Key, KeyMods, LogicalKey};

/// Map one of our `Key` events to a (keydown, keyup) pair of Servo keyboard
/// events. Returns `None` for keys that have no Servo equivalent.
pub fn map_keyboard_event(key: &Key) -> Option<(ServoKeyboardEvent, ServoKeyboardEvent)> {
    let (logical_key, code, mut extra_mods) = map_logical_key(key)?;
    extra_mods |= modifiers_from_mods(&key.modifiers);

    let make = |state| {
        ServoKeyboardEvent::new_without_event(
            state,
            logical_key.clone(),
            code,
            Location::Standard,
            extra_mods,
            false,
            false,
        )
    };

    Some((make(KeyState::Down), make(KeyState::Up)))
}

fn map_logical_key(key: &Key) -> Option<(ServoKey, Code, ServoModifiers)> {
    let empty = ServoModifiers::empty();

    match &key.logical {
        LogicalKey::Tab => Some((ServoKey::Named(NamedKey::Tab), Code::Tab, empty)),
        LogicalKey::Enter => Some((ServoKey::Named(NamedKey::Enter), Code::Enter, empty)),
        LogicalKey::Escape => Some((ServoKey::Named(NamedKey::Escape), Code::Escape, empty)),
        LogicalKey::Backspace | LogicalKey::Delete => {
            Some((ServoKey::Named(NamedKey::Backspace), Code::Backspace, empty))
        }
        LogicalKey::ArrowUp => Some((ServoKey::Named(NamedKey::ArrowUp), Code::ArrowUp, empty)),
        LogicalKey::ArrowDown => {
            Some((ServoKey::Named(NamedKey::ArrowDown), Code::ArrowDown, empty))
        }
        LogicalKey::ArrowRight => Some((
            ServoKey::Named(NamedKey::ArrowRight),
            Code::ArrowRight,
            empty,
        )),
        LogicalKey::ArrowLeft => {
            Some((ServoKey::Named(NamedKey::ArrowLeft), Code::ArrowLeft, empty))
        }

        LogicalKey::Char(c) => {
            // Ctrl+letter: encode as control character for Servo.
            let (ch, ctrl_mod) = if key.modifiers.ctrl && c.is_ascii_alphabetic() {
                (*c, ServoModifiers::CONTROL)
            } else {
                (*c, empty)
            };

            let code = character_code(ch)?;
            Some((ServoKey::Character(ch.to_string()), code, ctrl_mod))
        }
    }
}

fn character_code(ch: char) -> Option<Code> {
    Some(match ch.to_ascii_lowercase() {
        'a' => Code::KeyA,
        'b' => Code::KeyB,
        'c' => Code::KeyC,
        'd' => Code::KeyD,
        'e' => Code::KeyE,
        'f' => Code::KeyF,
        'g' => Code::KeyG,
        'h' => Code::KeyH,
        'i' => Code::KeyI,
        'j' => Code::KeyJ,
        'k' => Code::KeyK,
        'l' => Code::KeyL,
        'm' => Code::KeyM,
        'n' => Code::KeyN,
        'o' => Code::KeyO,
        'p' => Code::KeyP,
        'q' => Code::KeyQ,
        'r' => Code::KeyR,
        's' => Code::KeyS,
        't' => Code::KeyT,
        'u' => Code::KeyU,
        'v' => Code::KeyV,
        'w' => Code::KeyW,
        'x' => Code::KeyX,
        'y' => Code::KeyY,
        'z' => Code::KeyZ,
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        ' ' => Code::Space,
        '-' => Code::Minus,
        '=' => Code::Equal,
        '[' => Code::BracketLeft,
        ']' => Code::BracketRight,
        '\\' => Code::Backslash,
        ';' => Code::Semicolon,
        '\'' => Code::Quote,
        ',' => Code::Comma,
        '.' => Code::Period,
        '/' => Code::Slash,
        '`' => Code::Backquote,
        _ => return None,
    })
}

fn modifiers_from_mods(mods: &KeyMods) -> ServoModifiers {
    let mut m = ServoModifiers::empty();
    if mods.alt {
        m |= ServoModifiers::ALT;
    }
    if mods.ctrl {
        m |= ServoModifiers::CONTROL;
    }
    if mods.meta {
        m |= ServoModifiers::META;
    }
    if mods.shift {
        m |= ServoModifiers::SHIFT;
    }
    m
}
