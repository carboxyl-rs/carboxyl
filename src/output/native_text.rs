//! Terminal-native text overlay.
//!
//! After each layout pass Servo delivers a [`servo::DisplayList`] containing
//! every laid-out text run with its bounding rect and foreground color.
//! `TextOverlay` maps those runs onto terminal cells, sampling the pixel
//! buffer for a seamless background color.

use glam::Vec2;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};
use servo::{DisplayListItem, DisplayListItemContent, DisplayListItemSpace};
use std::collections::HashSet;
use unicode_width::UnicodeWidthStr;

use super::color::{LUMA_B, LUMA_G, LUMA_R, MAX_LUMA, to_terminal_color};

// ---------------------------------------------------------------------------
// Contrast constants
// ---------------------------------------------------------------------------

/// Minimum luma difference (on the 0..=MAX_LUMA pre-shift scale) required
/// to consider foreground/background contrast acceptable.
const MIN_CONTRAST: u32 = 6_000;

/// Luma threshold for the black-vs-white fallback. Half of MAX_LUMA.
const MID_LUMA: u32 = MAX_LUMA / 2;

/// Alpha threshold above which a SolidColor item is considered an opaque
/// occluder - anything on top with this alpha fully hides what's beneath.
const OCCLUDER_ALPHA: f32 = 0.9;

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Renders display-list text runs as native terminal glyphs.
///
/// Text cells replace the underlying pixel cells; the background color is
/// sampled from the pixel buffer so the result is visually seamless with the
/// surrounding pixel render.
pub struct TextOverlay<'a> {
    items: &'a [DisplayListItem],
    cell_pixels: Vec2,
    /// Raw RGBA8888 pixel data from the last frame, with frame dimensions.
    pixels: Option<(&'a [u8], u32, u32)>,
    true_color: bool,
    /// Live root viewport scroll offset in CSS pixels (from the painted frame).
    scroll_x: f32,
    scroll_y: f32,
}

impl<'a> TextOverlay<'a> {
    pub fn new(
        items: &'a [DisplayListItem],
        cell_pixels: Vec2,
        pixels: Option<(&'a [u8], u32, u32)>,
        true_color: bool,
        scroll_x: f32,
        scroll_y: f32,
    ) -> Self {
        Self {
            items,
            cell_pixels,
            pixels,
            true_color,
            scroll_x,
            scroll_y,
        }
    }

    /// Convert an item rect into viewport-relative CSS pixels using the live
    /// scroll offset for [`DisplayListItemSpace::Document`] items.
    fn viewport_rect(&self, item: &DisplayListItem) -> (f32, f32, f32, f32) {
        let (sx, sy) = match item.space {
            DisplayListItemSpace::Document => (self.scroll_x, self.scroll_y),
            DisplayListItemSpace::Viewport => (0.0, 0.0),
        };
        (
            item.rect.min.x - sx,
            item.rect.min.y - sy,
            item.rect.max.x - sx,
            item.rect.max.y - sy,
        )
    }
}

impl Widget for TextOverlay<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let viewport_w = area.width as f32 * self.cell_pixels.x;
        let viewport_h = area.height as f32 * self.cell_pixels.y;

        let mut occupied: HashSet<(u16, u16)> = HashSet::new();

        // Build occluder list while iterating front-to-back (reverse of paint
        // order). Items are in paint order (back→front); iterating in reverse
        // means we encounter the frontmost items first and accumulate their
        // rects so later (lower-z) text can be skipped when fully covered.
        let mut occluders: Vec<(f32, f32, f32, f32)> = Vec::new();

        for item in self.items.iter().rev() {
            let rect = self.viewport_rect(item);

            match &item.content {
                DisplayListItemContent::SolidColor { color } if color.a >= OCCLUDER_ALPHA => {
                    occluders.push(rect);
                }
                DisplayListItemContent::Image => {
                    occluders.push(rect);
                }
                DisplayListItemContent::SolidColor { .. } | DisplayListItemContent::Iframe { .. } => {
                    // Semi-transparent fills are not reliable occluders; iframe
                    // markers are containers (child items follow in paint order).
                }
                DisplayListItemContent::Text { text, color } => {
                    // Drop text fully covered by a higher-z opaque element.
                    if occluders
                        .iter()
                        .any(|&occ| fully_contains(occ, rect))
                    {
                        continue;
                    }

                    let (vx0, vy0, vx1, vy1) = rect;

                    // Skip runs outside the visible viewport.
                    if vx1 <= 0.0 || vy1 <= 0.0 || vx0 >= viewport_w || vy0 >= viewport_h {
                        continue;
                    }

                    let col = (vx0 / self.cell_pixels.x).floor();
                    let row = (vy0 / self.cell_pixels.y).floor();
                    if col < 0.0 || row < 0.0 {
                        continue;
                    }

                    let col = col as u16;
                    let row = row as u16;
                    if col >= area.width || row >= area.height {
                        continue;
                    }

                    let x = area.x + col;
                    let y = area.y + row;
                    let max_cols = (area.width - col) as usize;

                    let fg = Color::Rgb(
                        (color.r * 255.0) as u8,
                        (color.g * 255.0) as u8,
                        (color.b * 255.0) as u8,
                    );

                    let bg = self
                        .pixels
                        .and_then(|(px, pw, ph)| {
                            sample_cell_bg(px, pw, ph, col, row, self.cell_pixels)
                        })
                        .map(|c| to_terminal_color(c, self.true_color))
                        .unwrap_or(Color::Reset);

                    let fg = ensure_contrast(fg, bg);

                    let text = truncate_to_available(text, x, y, max_cols, &occupied);
                    if text.is_empty() {
                        continue;
                    }

                    let mut cursor = x;
                    for ch in text.chars() {
                        let w = ch.to_string().width() as u16;
                        for i in 0..w {
                            occupied.insert((cursor + i, y));
                        }
                        cursor += w;
                    }

                    buf.set_string(x, y, text, Style::new().fg(fg).bg(bg));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// True when `outer` fully covers `inner` in viewport coordinates.
fn fully_contains(
    outer: (f32, f32, f32, f32),
    inner: (f32, f32, f32, f32),
) -> bool {
    outer.0 <= inner.0 && outer.1 <= inner.1 && outer.2 >= inner.2 && outer.3 >= inner.3
}

fn truncate_to_available(
    s: &str,
    x: u16,
    y: u16,
    max_cols: usize,
    occupied: &HashSet<(u16, u16)>,
) -> String {
    let mut width = 0;
    let mut result = String::new();
    let mut cursor = x;
    for ch in s.chars() {
        let w = ch.to_string().width();
        if width + w > max_cols {
            break;
        }
        if occupied.contains(&(cursor, y)) {
            break;
        }
        result.push(ch);
        width += w;
        cursor += w as u16;
    }
    result
}

fn sample_cell_bg(
    pixels: &[u8],
    pw: u32,
    ph: u32,
    col: u16,
    row: u16,
    cell_pixels: Vec2,
) -> Option<(u8, u8, u8)> {
    let px = ((col as f32 + 0.5) * cell_pixels.x) as usize;
    let py = ((row as f32 + 0.5) * cell_pixels.y) as usize;

    let x = px.min(pw as usize - 1);
    let y = py.min(ph as usize - 1);
    let idx = (y * pw as usize + x) * 4;

    if idx + 2 >= pixels.len() {
        return None;
    }

    Some((pixels[idx], pixels[idx + 1], pixels[idx + 2]))
}

fn ensure_contrast(fg: Color, bg: Color) -> Color {
    let (fr, fg_g, fb) = rgb_of(fg);
    let (br, bg_g, bb) = rgb_of(bg);

    let fg_luma = fr as u32 * LUMA_R + fg_g as u32 * LUMA_G + fb as u32 * LUMA_B;
    let bg_luma = br as u32 * LUMA_R + bg_g as u32 * LUMA_G + bb as u32 * LUMA_B;

    if fg_luma.abs_diff(bg_luma) >= MIN_CONTRAST {
        return fg;
    }

    if bg_luma > MID_LUMA {
        Color::Black
    } else {
        Color::White
    }
}

fn rgb_of(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::Yellow => (170, 85, 0),
        Color::Blue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::Gray => (170, 170, 170),
        Color::DarkGray => (85, 85, 85),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (85, 255, 85),
        Color::LightYellow => (255, 255, 85),
        Color::LightBlue => (85, 85, 255),
        Color::LightMagenta => (255, 85, 255),
        Color::LightCyan => (85, 255, 255),
        Color::White => (255, 255, 255),
        Color::Indexed(_) | Color::Reset => (128, 128, 128),
    }
}
