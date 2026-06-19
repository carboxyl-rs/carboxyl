//! Terminal-native text overlay renderer.
//!
//! `TextOverlay` replaces the pixel cells where text exists with native
//! terminal glyphs, sampling the pixel buffer for the background color so
//! the result is visually seamless with the surrounding pixel render.

use glam::Vec2;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};
use unicode_width::UnicodeWidthChar;

use super::color::{LUMA_B, LUMA_G, LUMA_R, MAX_LUMA, to_terminal_color};

// ---------------------------------------------------------------------------
// Contrast constants
// ---------------------------------------------------------------------------

const MIN_CONTRAST: u32 = 6_000;
const MID_LUMA: u32 = MAX_LUMA / 2;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A single visible text item extracted from the page DOM.
#[derive(Clone, Debug)]
pub struct TextNode {
    pub text: String,
    /// Position in CSS pixels, viewport-relative (from getBoundingClientRect,
    /// adjusted by element padding+border to the content box origin).
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Foreground color from `getComputedStyle().color`.
    pub color: Color,
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Renders text nodes as native terminal glyphs, replacing the pixel cells
/// at each text position with a fully opaque cell whose background is sampled
/// from the pixel buffer. This makes native text visually seamless with the
/// surrounding pixel render while being crisp and resolution-independent.
pub struct TextOverlay<'a> {
    nodes: &'a [TextNode],
    cell_pixels: Vec2,
    /// Raw RGBA8888 pixel data from the last frame, with frame dimensions.
    pixels: Option<(&'a [u8], u32, u32)>,
    true_color: bool,
    /// Reusable scratch buffer for the occupied-cell bitmap; cleared and
    /// resized on each render to avoid a per-frame heap allocation.
    occupied: &'a mut Vec<bool>,
}

impl<'a> TextOverlay<'a> {
    pub fn new(
        nodes: &'a [TextNode],
        cell_pixels: Vec2,
        pixels: Option<(&'a [u8], u32, u32)>,
        true_color: bool,
        occupied: &'a mut Vec<bool>,
    ) -> Self {
        Self {
            nodes,
            cell_pixels,
            pixels,
            true_color,
            occupied,
        }
    }
}

impl Widget for TextOverlay<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let grid_w = area.width as usize;
        let grid_h = area.height as usize;

        // Reuse the caller-supplied buffer to avoid a per-frame allocation.
        // Earlier DOM nodes take priority (bitmap marks cells as occupied).
        self.occupied.clear();
        self.occupied.resize(grid_w * grid_h, false);

        for node in self.nodes {
            let col = (node.x / self.cell_pixels.x).floor() as u16;
            let row = (node.y / self.cell_pixels.y).floor() as u16;

            if col >= area.width || row >= area.height {
                continue;
            }

            let x = area.x + col;
            let y = area.y + row;
            let max_cols = (area.width - col) as usize;

            let bg = self
                .pixels
                .and_then(|(px, pw, ph)| sample_cell_bg(px, pw, ph, col, row, self.cell_pixels))
                .map(|c| to_terminal_color(c, self.true_color))
                .unwrap_or(Color::Reset);

            let fg = ensure_contrast(node.color, bg);

            let text = truncate_to_available(
                &node.text,
                col as usize,
                row as usize,
                max_cols,
                self.occupied,
                grid_w,
            );
            if text.is_empty() {
                continue;
            }

            let mut cur = col as usize;
            for ch in text.chars() {
                let w = ch.width().unwrap_or(0);
                for i in 0..w {
                    let idx = row as usize * grid_w + cur + i;
                    if idx < self.occupied.len() {
                        self.occupied[idx] = true;
                    }
                }
                cur += w;
            }

            buf.set_string(x, y, &text, Style::new().fg(fg).bg(bg));
        }
    }
}

fn truncate_to_available(
    s: &str,
    col: usize,
    row: usize,
    max_cols: usize,
    occupied: &[bool],
    grid_w: usize,
) -> String {
    let mut width = 0usize;
    let mut result = String::new();
    let mut cursor = col;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > max_cols {
            break;
        }
        if (0..w).any(|i| {
            let idx = row * grid_w + cursor + i;
            idx < occupied.len() && occupied[idx]
        }) {
            break;
        }
        result.push(ch);
        width += w;
        cursor += w;
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
