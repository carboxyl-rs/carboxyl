use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

/// The full state of the navigation bar. Owned by the main loop and passed
/// by reference to `NavWidget` each frame.
#[derive(Clone, Debug, Default)]
pub struct NavState {
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Cursor position within the URL field (byte index), or `None` when
    /// the field is not focused.
    pub cursor: Option<usize>,
}

impl NavState {
    /// Handle a key press directed at the nav bar. Returns the resolved
    /// action, or `NavAction::Ignore` if the key was consumed internally.
    pub fn keypress(&mut self, char: u8, alt: bool, meta: bool) -> NavAction {
        let modifier = if cfg!(target_os = "macos") { meta } else { alt };

        match self.cursor {
            None => match (modifier, char) {
                (true, 0x14) => NavAction::GoBack,
                (true, 0x13) => NavAction::GoForward,
                _ => NavAction::Forward,
            },
            Some(cursor) => {
                match char {
                    // Enter — navigate
                    0x0d => return NavAction::GoTo(self.url.clone()),
                    // Up — jump to start
                    0x11 => self.cursor = Some(0),
                    // Down — jump to end
                    0x12 => self.cursor = Some(self.url.width()),
                    // Right
                    0x13 => self.cursor = Some((cursor + 1).min(self.url.width())),
                    // Left
                    0x14 => self.cursor = Some(cursor.saturating_sub(1)),
                    // Backspace
                    0x7f
                        if cursor > 0 && cursor <= self.url.len() => {
                            self.url.remove(cursor - 1);
                            self.cursor = Some(cursor - 1);
                        }
                    // Printable ASCII
                    c if (0x20..0x7f).contains(&c) => {
                        self.url.insert(cursor, c as char);
                        self.cursor = Some((cursor + 1).min(self.url.width()));
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
            return NavAction::Forward;
        }

        self.cursor = None;

        match col {
            0..=2 => NavAction::GoBack,
            3..=5 => NavAction::GoForward,
            6..=8 => NavAction::Refresh,
            11.. => {
                let offset = (col as usize).saturating_sub(11);
                self.cursor = Some(offset.min(self.url.width()));
                NavAction::Ignore
            }
            _ => NavAction::Ignore,
        }
    }

    pub fn mouse_up(&mut self, _col: u16, row: u16) -> NavAction {
        if row != 0 {
            self.cursor = None;
            NavAction::Forward
        } else {
            NavAction::Ignore
        }
    }

    pub fn push(&mut self, url: &str, can_go_back: bool, can_go_forward: bool) {
        // Only reset the cursor if the URL actually changed while focused.
        if self.cursor.is_some() && self.url != url {
            self.cursor = Some(url.len());
        }
        self.url = url.to_owned();
        self.can_go_back = can_go_back;
        self.can_go_forward = can_go_forward;
    }
}

/// Actions the nav bar can produce in response to input.
#[derive(Debug)]
pub enum NavAction {
    Ignore,
    Forward,
    GoTo(String),
    GoBack,
    GoForward,
    Refresh,
}

/// Stateless ratatui widget — rendered each frame from `NavState`.
pub struct NavWidget<'a> {
    state: &'a NavState,
}

impl<'a> NavWidget<'a> {
    pub fn new(state: &'a NavState) -> Self {
        Self { state }
    }

    /// Returns the terminal cursor position if the URL field is focused,
    /// or `None` otherwise. Call this after `render` to set the cursor.
    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let col = 11 + self.state.cursor? as u16;
        Some((area.x + col.min(area.width.saturating_sub(1)), area.y))
    }
}

impl Widget for NavWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let style_active = Style::new().fg(Color::Black).bg(Color::White);
        let style_inactive = Style::new().fg(Color::DarkGray).bg(Color::White);
        let style_url = Style::new().fg(Color::Black).bg(Color::White);

        // Available width for the URL field (after the 3 buttons + brackets).
        let ui_prefix = 13u16; // "[‹][›][↻][ url... ]"
        let url_space = (area.width.saturating_sub(ui_prefix)) as usize;

        let url_display: String = self.state.url.chars().take(url_space).collect();
        let url_width = url_display.width();
        let padded = format!(" {}{} ", url_display, " ".repeat(url_space - url_width));

        let mut x = area.x;
        let y = area.y;

        let render_btn = |buf: &mut Buffer, x: &mut u16, label: &str, enabled: bool| {
            let style = if enabled {
                style_active
            } else {
                style_inactive
            };
            for ch in ["[", label, "]"] {
                let span = Span::styled(ch, style);
                let w = ch.width() as u16;
                buf.set_span(*x, y, &span, w);
                *x += w;
            }
        };

        render_btn(buf, &mut x, "\u{276e}", self.state.can_go_back);
        render_btn(buf, &mut x, "\u{276f}", self.state.can_go_forward);
        render_btn(buf, &mut x, "↻", true);
        render_btn(buf, &mut x, &padded, true);

        // Fill any remaining width (shouldn't happen but guards against
        // off-by-one in narrow terminals).
        while x < area.x + area.width {
            buf.cell_mut((x, y))
                .unwrap()
                .set_char(' ')
                .set_style(style_url);
            x += 1;
        }
    }
}
