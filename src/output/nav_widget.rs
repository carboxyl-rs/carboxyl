use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;
use url::Url;

use servo::{Key as ServoKey, Modifiers as ServoModifiers, NamedKey};

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

/// Column at which the URL input field begins.
///
/// Derived from button widths:
///   [‹] = 3 cols  (col 0–2)
///   [›] = 3 cols  (col 3–5)
///   [↻] = 3 cols  (col 6–8)
///   "  " separator = 2 cols  (col 9–10)
///
/// Total prefix = 11 cols.
const URL_FIELD_START: u16 = 11;

/// Hit-test regions for the navigation bar buttons (inclusive column ranges).
const BTN_BACK_COLS: std::ops::RangeInclusive<u16> = 0..=2;
const BTN_FORWARD_COLS: std::ops::RangeInclusive<u16> = 3..=5;
const BTN_RELOAD_COLS: std::ops::RangeInclusive<u16> = 6..=8;

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
    /// Cursor position within the URL field (char index).
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
                        self.cursor = Some(self.displayed_url().width());
                    }

                    ServoKey::Named(NamedKey::ArrowRight) => {
                        self.cursor = Some((cursor + 1).min(self.displayed_url().width()));
                    }

                    ServoKey::Named(NamedKey::ArrowLeft) => {
                        self.cursor = Some(cursor.saturating_sub(1));
                    }

                    ServoKey::Named(NamedKey::Backspace) if cursor > 0 => {
                        let buf = self
                            .staged
                            .get_or_insert_with(|| self.url.as_str().to_owned());

                        if cursor <= buf.len() {
                            buf.remove(cursor - 1);
                            self.cursor = Some(cursor - 1);
                        }
                    }

                    ServoKey::Named(NamedKey::Delete) => {
                        let buf = self
                            .staged
                            .get_or_insert_with(|| self.url.as_str().to_owned());

                        if cursor < buf.len() {
                            buf.remove(cursor);
                        }
                    }

                    ServoKey::Character(text) => {
                        if let Some(ch) = text.chars().next()
                            && !ch.is_control()
                        {
                            let buf = self
                                .staged
                                .get_or_insert_with(|| self.url.as_str().to_owned());

                            buf.insert(cursor, ch);

                            self.cursor = Some((cursor + 1).min(buf.width()));
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
            let offset = (col as usize).saturating_sub(URL_FIELD_START as usize);
            self.cursor = Some(offset.min(self.url.as_str().width()));
            // Populate the staging buffer so edits don't start from blank.
            self.staged = Some(self.url.as_str().to_owned());
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

pub struct NavWidget<'a> {
    state: &'a NavState,
}

impl<'a> NavWidget<'a> {
    pub fn new(state: &'a NavState) -> Self {
        Self { state }
    }

    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let col = URL_FIELD_START + self.state.cursor? as u16;
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

        let url_space = (area.width.saturating_sub(URL_FIELD_START + 2)) as usize;
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
