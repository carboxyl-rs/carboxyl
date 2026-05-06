use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

/// Full state of the navigation bar, owned by the main loop.
#[derive(Clone, Debug, Default)]
pub struct NavState {
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Cursor position within the URL field (char index), or `None` when
    /// the field is not focused.
    pub cursor: Option<usize>,
}

impl NavState {
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
                    0x0d => return NavAction::GoTo(self.url.clone()),
                    0x11 => self.cursor = Some(0),
                    0x12 => self.cursor = Some(self.url.width()),
                    0x13 => self.cursor = Some((cursor + 1).min(self.url.width())),
                    0x14 => self.cursor = Some(cursor.saturating_sub(1)),
                    0x7f if cursor > 0 && cursor <= self.url.len() => {
                        self.url.remove(cursor - 1);
                        self.cursor = Some(cursor - 1);
                    }
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
        if self.cursor.is_some() && self.url != url {
            self.cursor = Some(url.len());
        }
        self.url = url.to_owned();
        self.can_go_back = can_go_back;
        self.can_go_forward = can_go_forward;
    }
}

#[derive(Debug)]
pub enum NavAction {
    Ignore,
    Forward,
    GoTo(String),
    GoBack,
    GoForward,
    Refresh,
}

pub struct NavWidget<'a> {
    state: &'a NavState,
}

impl<'a> NavWidget<'a> {
    pub fn new(state: &'a NavState) -> Self {
        Self { state }
    }

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

        let active = Style::new().fg(Color::Black).bg(Color::White);
        let inactive = Style::new().fg(Color::DarkGray).bg(Color::White);

        let url_space = (area.width.saturating_sub(13)) as usize;
        let url_display: String = self.state.url.chars().take(url_space).collect();
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

        btn(buf, &mut x, "\u{276e}", self.state.can_go_back);
        btn(buf, &mut x, "\u{276f}", self.state.can_go_forward);
        btn(buf, &mut x, "↻", true);
        btn(buf, &mut x, &padded, true);

        while x < area.x + area.width {
            buf.cell_mut((x, y))
                .unwrap()
                .set_char(' ')
                .set_style(active);
            x += 1;
        }
    }
}
