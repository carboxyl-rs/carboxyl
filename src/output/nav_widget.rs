use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::Widget,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use url::Url;

use servo::{Key as ServoKey, Modifiers as ServoModifiers, NamedKey};

// ---------------------------------------------------------------------------
// Button layout constants
// ---------------------------------------------------------------------------
//
// The navigation bar is laid out left-to-right as:
//
//   [‹]  [›]  [↻]  <space><space>  [ url field ]
//    0-2  3-5  6-8  9-10            11 …
//
// Each button is three columns wide: "[", glyph, "]".

const BTN_WIDTH: u16 = 3;

const BTN_BACK_START: u16 = 0;
const BTN_FORWARD_START: u16 = BTN_BACK_START + BTN_WIDTH;
const BTN_RELOAD_START: u16 = BTN_FORWARD_START + BTN_WIDTH;

const BTN_BACK_COLS: std::ops::RangeInclusive<u16> =
    BTN_BACK_START..=BTN_BACK_START + BTN_WIDTH - 1;
const BTN_FORWARD_COLS: std::ops::RangeInclusive<u16> =
    BTN_FORWARD_START..=BTN_FORWARD_START + BTN_WIDTH - 1;
const BTN_RELOAD_COLS: std::ops::RangeInclusive<u16> =
    BTN_RELOAD_START..=BTN_RELOAD_START + BTN_WIDTH - 1;

/// Two-column spacer between the reload button and the URL field.
const URL_FIELD_PADDING: u16 = 2;

/// Column at which the URL input field begins.
const URL_FIELD_START: u16 = BTN_RELOAD_START + BTN_WIDTH + URL_FIELD_PADDING;

/// Padding columns consumed by the URL field's own leading and trailing spaces.
const URL_FIELD_INNER_PADDING: u16 = 2;

// ---------------------------------------------------------------------------
// Navigation capability flags
// ---------------------------------------------------------------------------

/// Which history directions are currently navigable.
#[derive(Clone, Copy, Debug, Default)]
pub struct NavigationCapability {
    pub back: bool,
    pub forward: bool,
}

impl NavigationCapability {
    pub fn can_go_back(self) -> bool {
        self.back
    }

    pub fn can_go_forward(self) -> bool {
        self.forward
    }
}

// ---------------------------------------------------------------------------
// NavState
// ---------------------------------------------------------------------------

/// Full state of the navigation bar, owned by the main loop.
#[derive(Clone, Debug)]
pub struct NavState {
    /// The committed, validated URL shown in the address bar.
    pub url: Url,
    /// In-flight edit buffer while the user is typing; `None` when the
    /// address bar is not focused.
    pub staged: Option<String>,
    pub nav: NavigationCapability,
    /// Byte offset into the staged (or committed URL) string.
    /// `None` when the address bar is not focused.
    pub cursor: Option<usize>,
}

impl Default for NavState {
    fn default() -> Self {
        Self {
            // about:blank is always valid — unwrap is safe.
            url: Url::parse("about:blank").unwrap(),
            staged: None,
            nav: NavigationCapability::default(),
            cursor: None,
        }
    }
}

impl NavState {
    /// Returns the string currently shown in the address bar:
    /// the staged edit if one is in progress, otherwise the committed URL.
    fn displayed_url(&self) -> &str {
        self.staged.as_deref().unwrap_or(self.url.as_str())
    }

    pub fn keyboard(&mut self, key: &ServoKey, modifiers: ServoModifiers) -> NavAction {
        let modifier = if cfg!(target_os = "macos") {
            modifiers.contains(ServoModifiers::META)
        } else {
            modifiers.contains(ServoModifiers::ALT)
        };

        match self.cursor {
            None => match (modifier, key) {
                (true, ServoKey::Named(NamedKey::ArrowLeft)) => NavAction::GoBack,
                (true, ServoKey::Named(NamedKey::ArrowRight)) => NavAction::GoForward,
                _ => NavAction::Forward,
            },

            Some(cursor) => {
                match key {
                    ServoKey::Named(NamedKey::Enter) => {
                        let raw = self.displayed_url().to_owned();
                        self.staged = None;
                        self.cursor = None;
                        return NavAction::GoTo(raw);
                    }

                    ServoKey::Named(NamedKey::ArrowUp) => {
                        self.cursor = Some(0);
                    }

                    ServoKey::Named(NamedKey::ArrowDown) => {
                        self.cursor = Some(self.displayed_url().len());
                    }

                    ServoKey::Named(NamedKey::ArrowRight) => {
                        let s = self.displayed_url();
                        // Advance by exactly one Unicode scalar, not one byte.
                        let new = s[cursor..]
                            .chars()
                            .next()
                            .map(|ch| cursor + ch.len_utf8())
                            .unwrap_or(s.len());
                        self.cursor = Some(new);
                    }

                    ServoKey::Named(NamedKey::ArrowLeft) => {
                        // Retreat to the start of the preceding Unicode scalar.
                        let new = self.displayed_url()[..cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.cursor = Some(new);
                    }

                    ServoKey::Named(NamedKey::Backspace) if cursor > 0 => {
                        let buf = self
                            .staged
                            .get_or_insert_with(|| self.url.as_str().to_owned());

                        // Find and remove the whole char ending at `cursor`.
                        let clamped = cursor.min(buf.len());
                        if let Some((prev, _)) = buf[..clamped].char_indices().next_back() {
                            buf.drain(prev..clamped);
                            self.cursor = Some(prev);
                        }
                    }

                    ServoKey::Named(NamedKey::Delete) => {
                        let buf = self
                            .staged
                            .get_or_insert_with(|| self.url.as_str().to_owned());

                        // Remove the whole char starting at `cursor`.
                        if cursor < buf.len() {
                            let ch_len = buf[cursor..]
                                .chars()
                                .next()
                                .map(|c| c.len_utf8())
                                .unwrap_or(1);
                            buf.drain(cursor..cursor + ch_len);
                        }
                    }

                    ServoKey::Character(text) => {
                        if let Some(ch) = text.chars().next()
                            && !ch.is_control()
                        {
                            let buf = self
                                .staged
                                .get_or_insert_with(|| self.url.as_str().to_owned());

                            // Insert at the nearest valid char boundary.
                            let pos = cursor.min(buf.len());
                            buf.insert(pos, ch);
                            self.cursor = Some(pos + ch.len_utf8());
                        }
                    }

                    _ => {}
                }

                NavAction::Ignore
            }
        }
    }

    pub fn mouse_down(&mut self, col: u16, row: u16) -> NavAction {
        if row != 0 {
            self.cursor = None;
            self.staged = None;
            return NavAction::Forward;
        }

        self.cursor = None;
        self.staged = None;

        if BTN_BACK_COLS.contains(&col) {
            return NavAction::GoBack;
        }
        if BTN_FORWARD_COLS.contains(&col) {
            return NavAction::GoForward;
        }
        if BTN_RELOAD_COLS.contains(&col) {
            return NavAction::Refresh;
        }
        if col >= URL_FIELD_START {
            let col_offset = (col as usize).saturating_sub(URL_FIELD_START as usize);
            let url_str = self.url.as_str();
            // Convert visual column offset to the byte position within the string.
            self.cursor = Some(col_to_byte_offset(url_str, col_offset));
            self.staged = Some(url_str.to_owned());
            return NavAction::Ignore;
        }

        NavAction::Ignore
    }

    pub fn mouse_up(&mut self, _col: u16, row: u16) -> NavAction {
        if row != 0 {
            self.cursor = None;
            self.staged = None;
            NavAction::Forward
        } else {
            NavAction::Ignore
        }
    }

    /// Update the committed URL and navigation capability after a navigation event.
    pub fn push(&mut self, url: Url, nav: NavigationCapability) {
        // If there's an active staged edit that differs from the incoming URL,
        // move the cursor to end-of-field so the user sees the new URL fully.
        if self.cursor.is_some() && self.url != url {
            self.cursor = Some(url.as_str().len());
        }
        self.url = url;
        self.staged = None;
        self.nav = nav;
    }
}

// ---------------------------------------------------------------------------
// NavAction
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum NavAction {
    Ignore,
    Forward,
    /// Raw string entered by the user; caller is responsible for URL parsing.
    GoTo(String),
    GoBack,
    GoForward,
    Refresh,
}

// ---------------------------------------------------------------------------
// NavWidget
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct NavWidget<'a> {
    state: &'a NavState,
}

impl<'a> NavWidget<'a> {
    pub fn new(state: &'a NavState) -> Self {
        Self { state }
    }

    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let byte_pos = self.state.cursor?;
        let s = self
            .state
            .staged
            .as_deref()
            .unwrap_or(self.state.url.as_str());
        // Convert byte offset back to display columns for terminal cursor placement.
        let display_col = s[..byte_pos.min(s.len())].width() as u16;
        let col = URL_FIELD_START + display_col;
        Some((area.x + col.min(area.width.saturating_sub(1)), area.y))
    }
}

impl Widget for NavWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let active = Style::new().fg(Color::Black).bg(Color::White);
        let inactive = Style::new().fg(Color::DarkGray).bg(Color::White);

        let url_space = (area
            .width
            .saturating_sub(URL_FIELD_START + URL_FIELD_INNER_PADDING))
            as usize;
        let displayed = self.state.displayed_url();
        let url_display: String = displayed.chars().take(url_space).collect();
        let url_width = url_display.width();
        let padded = format!(" {}{} ", url_display, " ".repeat(url_space - url_width));

        let mut x = area.x;
        let y = area.y;

        let btn = |buf: &mut Buffer, x: &mut u16, label: &str, enabled: bool| {
            let style = if enabled { active } else { inactive };
            for part in ["[", label, "]"] {
                let w = part.width() as u16;
                buf.set_span(*x, y, &Span::styled(part, style), w);
                *x += w;
            }
        };

        btn(buf, &mut x, "‹", self.state.nav.can_go_back());
        btn(buf, &mut x, "›", self.state.nav.can_go_forward());
        btn(buf, &mut x, "↻", true);
        btn(buf, &mut x, &padded, true);

        for x in x..area.x + area.width {
            buf[(x, y)].set_char(' ').set_style(active);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a visual-column offset within `s` to the corresponding byte index.
/// If `col` exceeds the string's display width, returns `s.len()`.
fn col_to_byte_offset(s: &str, col: usize) -> usize {
    let mut width = 0;
    for (byte_pos, ch) in s.char_indices() {
        if width >= col {
            return byte_pos;
        }
        width += ch.width().unwrap_or(0);
    }
    s.len()
}
