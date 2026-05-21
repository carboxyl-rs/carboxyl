use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use servo::{
    Code, Key as ServoKey, KeyState, KeyboardEvent as ServoKeyboardEvent, Location,
    Modifiers as ServoModifiers, NamedKey,
};

pub fn map_keyboard_event(event: KeyEvent) -> Option<ServoKeyboardEvent> {
    let modifiers = effective_modifiers(&event);

    let (key, code, location) = map_key_code(&event.code);

    let state = match event.kind {
        KeyEventKind::Press | KeyEventKind::Repeat => KeyState::Down,
        KeyEventKind::Release => KeyState::Up,
    };

    Some(ServoKeyboardEvent::new_without_event(
        state,
        key,
        code,
        location,
        map_modifiers(modifiers),
        matches!(event.kind, KeyEventKind::Repeat),
        false,
    ))
}

fn effective_modifiers(event: &KeyEvent) -> KeyModifiers {
    let mut mods = event.modifiers;

    if matches!(event.code, KeyCode::BackTab) {
        mods |= KeyModifiers::SHIFT;
    }

    mods
}

fn map_key_code(code: &KeyCode) -> (ServoKey, Code, Location) {
    match code {
        KeyCode::Backspace => (
            ServoKey::Named(NamedKey::Backspace),
            Code::Backspace,
            Location::Standard,
        ),

        KeyCode::Enter => (
            ServoKey::Named(NamedKey::Enter),
            Code::Enter,
            Location::Standard,
        ),

        KeyCode::Left => (
            ServoKey::Named(NamedKey::ArrowLeft),
            Code::ArrowLeft,
            Location::Standard,
        ),

        KeyCode::Right => (
            ServoKey::Named(NamedKey::ArrowRight),
            Code::ArrowRight,
            Location::Standard,
        ),

        KeyCode::Up => (
            ServoKey::Named(NamedKey::ArrowUp),
            Code::ArrowUp,
            Location::Standard,
        ),

        KeyCode::Down => (
            ServoKey::Named(NamedKey::ArrowDown),
            Code::ArrowDown,
            Location::Standard,
        ),

        KeyCode::Home => (
            ServoKey::Named(NamedKey::Home),
            Code::Home,
            Location::Standard,
        ),

        KeyCode::End => (
            ServoKey::Named(NamedKey::End),
            Code::End,
            Location::Standard,
        ),

        KeyCode::PageUp => (
            ServoKey::Named(NamedKey::PageUp),
            Code::PageUp,
            Location::Standard,
        ),

        KeyCode::PageDown => (
            ServoKey::Named(NamedKey::PageDown),
            Code::PageDown,
            Location::Standard,
        ),

        KeyCode::Tab => (
            ServoKey::Named(NamedKey::Tab),
            Code::Tab,
            Location::Standard,
        ),

        KeyCode::BackTab => (
            ServoKey::Named(NamedKey::Tab),
            Code::Tab,
            Location::Standard,
        ),

        KeyCode::Delete => (
            ServoKey::Named(NamedKey::Delete),
            Code::Delete,
            Location::Standard,
        ),

        KeyCode::Insert => (
            ServoKey::Named(NamedKey::Insert),
            Code::Insert,
            Location::Standard,
        ),

        KeyCode::Esc => (
            ServoKey::Named(NamedKey::Escape),
            Code::Escape,
            Location::Standard,
        ),

        KeyCode::CapsLock => (
            ServoKey::Named(NamedKey::CapsLock),
            Code::CapsLock,
            Location::Standard,
        ),

        KeyCode::ScrollLock => (
            ServoKey::Named(NamedKey::ScrollLock),
            Code::ScrollLock,
            Location::Standard,
        ),

        KeyCode::NumLock => (
            ServoKey::Named(NamedKey::NumLock),
            Code::NumLock,
            Location::Standard,
        ),

        KeyCode::PrintScreen => (
            ServoKey::Named(NamedKey::PrintScreen),
            Code::PrintScreen,
            Location::Standard,
        ),

        KeyCode::Pause => (
            ServoKey::Named(NamedKey::Pause),
            Code::Pause,
            Location::Standard,
        ),

        KeyCode::Menu => (
            ServoKey::Named(NamedKey::ContextMenu),
            Code::ContextMenu,
            Location::Standard,
        ),

        KeyCode::KeypadBegin => (
            ServoKey::Named(NamedKey::Clear),
            Code::Numpad5,
            Location::Numpad,
        ),

        KeyCode::Char(c) => {
            let key = ServoKey::Character(c.to_string());

            let code = character_code(*c).unwrap_or(Code::Unidentified);

            (key, code, Location::Standard)
        }

        KeyCode::F(n) => (
            ServoKey::Named(function_named_key(*n)),
            function_code(*n),
            Location::Standard,
        ),

        KeyCode::Modifier(modifier) => match modifier {
            crossterm::event::ModifierKeyCode::LeftShift => (
                ServoKey::Named(NamedKey::Shift),
                Code::ShiftLeft,
                Location::Left,
            ),

            crossterm::event::ModifierKeyCode::RightShift => (
                ServoKey::Named(NamedKey::Shift),
                Code::ShiftRight,
                Location::Right,
            ),

            crossterm::event::ModifierKeyCode::LeftControl => (
                ServoKey::Named(NamedKey::Control),
                Code::ControlLeft,
                Location::Left,
            ),

            crossterm::event::ModifierKeyCode::RightControl => (
                ServoKey::Named(NamedKey::Control),
                Code::ControlRight,
                Location::Right,
            ),

            crossterm::event::ModifierKeyCode::LeftAlt => (
                ServoKey::Named(NamedKey::Alt),
                Code::AltLeft,
                Location::Left,
            ),

            crossterm::event::ModifierKeyCode::RightAlt => (
                ServoKey::Named(NamedKey::Alt),
                Code::AltRight,
                Location::Right,
            ),

            crossterm::event::ModifierKeyCode::LeftSuper
            | crossterm::event::ModifierKeyCode::LeftHyper
            | crossterm::event::ModifierKeyCode::LeftMeta => (
                ServoKey::Named(NamedKey::Meta),
                Code::MetaLeft,
                Location::Left,
            ),

            crossterm::event::ModifierKeyCode::RightSuper
            | crossterm::event::ModifierKeyCode::RightHyper
            | crossterm::event::ModifierKeyCode::RightMeta => (
                ServoKey::Named(NamedKey::Meta),
                Code::MetaRight,
                Location::Right,
            ),

            crossterm::event::ModifierKeyCode::IsoLevel3Shift
            | crossterm::event::ModifierKeyCode::IsoLevel5Shift => (
                ServoKey::Named(NamedKey::AltGraph),
                Code::AltRight,
                Location::Right,
            ),
        },

        KeyCode::Media(media) => (
            ServoKey::Character(format!("{media:?}")),
            Code::Unidentified,
            Location::Standard,
        ),

        KeyCode::Null => (
            ServoKey::Named(NamedKey::Unidentified),
            Code::Unidentified,
            Location::Standard,
        ),
    }
}

fn map_modifiers(mods: KeyModifiers) -> ServoModifiers {
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

fn character_code(ch: char) -> Option<Code> {
    Some(match ch {
        'a' | 'A' => Code::KeyA,
        'b' | 'B' => Code::KeyB,
        'c' | 'C' => Code::KeyC,
        'd' | 'D' => Code::KeyD,
        'e' | 'E' => Code::KeyE,
        'f' | 'F' => Code::KeyF,
        'g' | 'G' => Code::KeyG,
        'h' | 'H' => Code::KeyH,
        'i' | 'I' => Code::KeyI,
        'j' | 'J' => Code::KeyJ,
        'k' | 'K' => Code::KeyK,
        'l' | 'L' => Code::KeyL,
        'm' | 'M' => Code::KeyM,
        'n' | 'N' => Code::KeyN,
        'o' | 'O' => Code::KeyO,
        'p' | 'P' => Code::KeyP,
        'q' | 'Q' => Code::KeyQ,
        'r' | 'R' => Code::KeyR,
        's' | 'S' => Code::KeyS,
        't' | 'T' => Code::KeyT,
        'u' | 'U' => Code::KeyU,
        'v' | 'V' => Code::KeyV,
        'w' | 'W' => Code::KeyW,
        'x' | 'X' => Code::KeyX,
        'y' | 'Y' => Code::KeyY,
        'z' | 'Z' => Code::KeyZ,

        '0' | ')' => Code::Digit0,
        '1' | '!' => Code::Digit1,
        '2' | '@' => Code::Digit2,
        '3' | '#' => Code::Digit3,
        '4' | '$' => Code::Digit4,
        '5' | '%' => Code::Digit5,
        '6' | '^' => Code::Digit6,
        '7' | '&' => Code::Digit7,
        '8' | '*' => Code::Digit8,
        '9' | '(' => Code::Digit9,

        ' ' => Code::Space,

        '-' | '_' => Code::Minus,
        '=' | '+' => Code::Equal,

        '[' | '{' => Code::BracketLeft,
        ']' | '}' => Code::BracketRight,

        '\\' | '|' => Code::Backslash,

        ';' | ':' => Code::Semicolon,
        '\'' | '"' => Code::Quote,

        ',' | '<' => Code::Comma,
        '.' | '>' => Code::Period,

        '/' | '?' => Code::Slash,

        '`' | '~' => Code::Backquote,

        _ => return None,
    })
}

fn function_named_key(n: u8) -> NamedKey {
    match n {
        1 => NamedKey::F1,
        2 => NamedKey::F2,
        3 => NamedKey::F3,
        4 => NamedKey::F4,
        5 => NamedKey::F5,
        6 => NamedKey::F6,
        7 => NamedKey::F7,
        8 => NamedKey::F8,
        9 => NamedKey::F9,
        10 => NamedKey::F10,
        11 => NamedKey::F11,
        12 => NamedKey::F12,
        13 => NamedKey::F13,
        14 => NamedKey::F14,
        15 => NamedKey::F15,
        16 => NamedKey::F16,
        17 => NamedKey::F17,
        18 => NamedKey::F18,
        19 => NamedKey::F19,
        20 => NamedKey::F20,
        21 => NamedKey::F21,
        22 => NamedKey::F22,
        23 => NamedKey::F23,
        24 => NamedKey::F24,
        _ => NamedKey::Unidentified,
    }
}

fn function_code(n: u8) -> Code {
    match n {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        13 => Code::F13,
        14 => Code::F14,
        15 => Code::F15,
        16 => Code::F16,
        17 => Code::F17,
        18 => Code::F18,
        19 => Code::F19,
        20 => Code::F20,
        21 => Code::F21,
        22 => Code::F22,
        23 => Code::F23,
        24 => Code::F24,
        _ => Code::Unidentified,
    }
}
