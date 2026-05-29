//! Terminal-native text overlay.
//!
//! After each page load and on scroll/resize, the Servo thread evaluates a
//! JavaScript snippet that collects all visible text nodes (including input
//! field values) with their bounding boxes and computed styles. The results
//! are forwarded to the main loop as `RuntimeEvent::TextNodes`.
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
use servo::JSValue;
use std::collections::HashSet;
use unicode_width::UnicodeWidthStr;

use super::color::{LUMA_B, LUMA_G, LUMA_R, MAX_LUMA, to_terminal_color};

// ---------------------------------------------------------------------------
// Contrast constants
// ---------------------------------------------------------------------------

/// Minimum luma difference (on the 0..=MAX_LUMA pre-shift scale) required
/// to consider foreground/background contrast acceptable.
/// Empirically tuned: small enough not to reject valid dim colors, large
/// enough to prevent near-invisible text.
const MIN_CONTRAST: u32 = 6_000;

/// Luma threshold for the black-vs-white fallback. Half of MAX_LUMA.
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
    /// Foreground color from `getComputedStyle().color` read while the
    /// suppress stylesheet is temporarily disabled.
    pub color: Color,
}

/// Parse a `JSValue::Array` of objects returned by the extraction script.
pub fn parse_js_nodes(value: &JSValue) -> Vec<TextNode> {
    let JSValue::Array(items) = value else {
        return vec![];
    };

    items
        .iter()
        .filter_map(|item| {
            let JSValue::Object(map) = item else {
                return None;
            };

            let text = str_field(map, "t")?.trim().to_owned();
            if text.is_empty() {
                return None;
            }

            let x = f32_field(map, "x")?;
            let y = f32_field(map, "y")?;
            let w = f32_field(map, "w")?;
            let h = f32_field(map, "h")?;

            if w <= 0.0 || h <= 0.0 || x < 0.0 || y < 0.0 {
                return None;
            }

            let color = str_field(map, "c")
                .and_then(parse_css_color)
                .unwrap_or(Color::Reset);

            Some(TextNode {
                text,
                x,
                y,
                width: w,
                height: h,
                color,
            })
        })
        .collect()
}

fn str_field<'a>(
    map: &'a std::collections::HashMap<String, JSValue>,
    key: &str,
) -> Option<&'a str> {
    match map.get(key)? {
        JSValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

fn f32_field(map: &std::collections::HashMap<String, JSValue>, key: &str) -> Option<f32> {
    match map.get(key)? {
        JSValue::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// Parse CSS `rgb(r, g, b)` or `rgba(r, g, b, a)`.
///
/// The extraction script temporarily disables the suppress stylesheet before
/// reading `getComputedStyle().color`, so the values here are the page's
/// authored colors. Alpha is ignored — semi-transparent text is rendered
/// opaque in the terminal.
fn parse_css_color(s: &str) -> Option<Color> {
    let inner = s
        .trim()
        .strip_prefix("rgba(")
        .or_else(|| s.trim().strip_prefix("rgb("))?
        .strip_suffix(')')?;

    let mut parts = inner.split(',').map(|p| p.trim());
    let r: u8 = parts.next()?.parse().ok()?;
    let g: u8 = parts.next()?.parse().ok()?;
    let b: u8 = parts.next()?.parse().ok()?;

    Some(Color::Rgb(r, g, b))
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
}

impl<'a> TextOverlay<'a> {
    pub fn new(
        nodes: &'a [TextNode],
        cell_pixels: Vec2,
        pixels: Option<(&'a [u8], u32, u32)>,
        true_color: bool,
    ) -> Self {
        Self {
            nodes,
            cell_pixels,
            pixels,
            true_color,
        }
    }
}

impl Widget for TextOverlay<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Terminal cells written this frame — (absolute_x, absolute_y).
        // Earlier nodes in DOM order take priority; later nodes truncate
        // at the first occupied cell so they never overwrite.
        let mut occupied: HashSet<(u16, u16)> = HashSet::new();

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

            // Truncate to area edge AND to first occupied cell.
            let text = truncate_to_available(&node.text, x, y, max_cols, &occupied);
            if text.is_empty() {
                continue;
            }

            // Mark cells occupied before the next node runs.
            let mut cursor = x;
            for ch in text.chars() {
                let w = ch.to_string().width() as u16;
                for i in 0..w {
                    occupied.insert((cursor + i, y));
                }
                cursor += w;
            }

            buf.set_string(x, y, &text, Style::new().fg(fg).bg(bg));
        }
    }
}

/// Like `truncate_to_width`, but also stops at the first cell already
/// claimed by a previously rendered node.
///
/// Wide characters (e.g. CJK glyphs, width = 2) occupy two consecutive
/// cells. We reject them if ANY of those cells is already occupied —
/// checking only the first cell would silently overwrite the second.
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
        // Check every cell the character occupies, not just the first.
        if (0..w as u16).any(|i| occupied.contains(&(cursor + i, y))) {
            break;
        }
        result.push(ch);
        width += w;
        cursor += w as u16;
    }
    result
}

/// Sample the center pixel of a terminal cell from the RGBA8888 pixel buffer.
fn sample_cell_bg(
    pixels: &[u8],
    pw: u32,
    ph: u32,
    col: u16,
    row: u16,
    cell_pixels: Vec2,
) -> Option<(u8, u8, u8)> {
    // Center of the cell in pixel coordinates.
    let px = ((col as f32 + 0.5) * cell_pixels.x) as usize;
    let py = ((row as f32 + 0.5) * cell_pixels.y) as usize;

    let x = px.min(pw as usize - 1);
    let y = py.min(ph as usize - 1);
    let idx = (y * pw as usize + x) * 4; // ×4 for RGBA bytes per pixel

    if idx + 2 >= pixels.len() {
        return None;
    }

    Some((pixels[idx], pixels[idx + 1], pixels[idx + 2]))
}

/// Ensure adequate contrast between `fg` and `bg`.
/// Falls back to pure black or white based on background luminance when the
/// two colors are too close to be readable.
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

/// Decompose any ratatui [`Color`] into its `(r, g, b)` triple.
///
/// Named terminal colors are mapped to their conventional ANSI RGB values.
/// `Color::Reset` and indexed colors outside the 6×6×6 cube are treated as
/// mid-gray, keeping contrast logic conservative rather than silent.
fn rgb_of(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),

        // Standard ANSI named colors (conventional sRGB approximations).
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

        // Indexed and Reset: conservatively mid-gray so contrast check errs
        // toward the black/white fallback rather than passing an unknown color.
        Color::Indexed(_) | Color::Reset => (128, 128, 128),
    }
}

// ---------------------------------------------------------------------------
// JavaScript integration
// ---------------------------------------------------------------------------

/// Injected once per page load (on `LoadStatus::HeadParsed`) to make all text
/// transparent in Servo's pixel render. Layout and bounding boxes are fully
/// preserved — only the paint color changes — so `EXTRACTION_SCRIPT` still
/// returns accurate positions for the native terminal text overlay.
///
/// A sentinel attribute (`data-carboxyl-suppress`) guards against duplicate
/// injection on pages that fire multiple load-complete notifications.
pub const SUPPRESS_TEXT_SCRIPT: &str = include_str!("suppress.js");

/// Evaluates on the page and returns an Array of Objects with fields:
/// `t` (text), `x`, `y`, `w`, `h` (viewport-relative CSS px, content-box
/// origin), `c` (CSS color read with suppression temporarily disabled).
///
/// Covers:
/// - Regular text nodes via TreeWalker
/// - `<button>` and `<select>` elements
/// - `<input>` and `<textarea>` values
/// - `[contenteditable]` elements
/// - Filters: off-screen, zero-size, hidden, occluded, whitespace-only
pub const EXTRACTION_SCRIPT: &str = include_str!("extract.js");
