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
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A single visible text item extracted from the page DOM.
#[derive(Clone, Debug)]
pub struct TextNode {
    pub text: String,
    /// Position in CSS pixels, viewport-relative (from getBoundingClientRect).
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Foreground color from `getComputedStyle().color`.
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

            if w <= 0.0 || h <= 0.0 || y < 0.0 {
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
/// After text suppression, computed color will be `rgba(r, g, b, 0)` —
/// the RGB channels still carry the original color, so we read them as-is
/// and discard the alpha component entirely.
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
        for node in self.nodes {
            // Use the tight text y directly — the JS now gives us the Range
            // bounding box rather than the parent element box, so no heuristic
            // offset is needed.
            let col = (node.x / self.cell_pixels.x).floor() as u16;
            let row = (node.y / self.cell_pixels.y).floor() as u16;

            if col >= area.width || row >= area.height {
                continue;
            }

            let x = area.x + col;
            let y = area.y + row;
            let max_cols = (area.width - col) as usize;

            // Sample background from pixel buffer at this cell's center.
            let bg = self
                .pixels
                .and_then(|(px, pw, ph)| sample_cell_bg(px, pw, ph, col, row, self.cell_pixels))
                .map(|c| to_terminal_color(c, self.true_color))
                .unwrap_or(Color::Reset);

            let fg = ensure_contrast(node.color, bg);

            let text = truncate_to_width(&node.text, max_cols);
            buf.set_string(x, y, &text, Style::new().fg(fg).bg(bg));
        }
    }
}

/// Sample the average color of a terminal cell from the pixel buffer.
/// Each cell covers `cell_pixels.x` × `cell_pixels.y` pixels — we sample
/// the center pixel for speed.
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

/// Convert an RGB triple to a ratatui `Color` based on terminal capability.
fn to_terminal_color((r, g, b): (u8, u8, u8), true_color: bool) -> Color {
    if true_color {
        Color::Rgb(r, g, b)
    } else {
        let q = |v: u8| (v / 43).min(5);
        Color::Indexed(16 + q(r) * 36 + q(g) * 6 + q(b))
    }
}

/// Ensure adequate contrast between fg and bg.
/// Falls back to black or white based on background luminance.
fn ensure_contrast(fg: Color, bg: Color) -> Color {
    let (fr, fg_c, fb) = rgb_of(fg);
    let (br, bg_c, bb) = rgb_of(bg);

    let fg_luma = fr as u32 * 77 + fg_c as u32 * 150 + fb as u32 * 29;
    let bg_luma = br as u32 * 77 + bg_c as u32 * 150 + bb as u32 * 29;

    if fg_luma.abs_diff(bg_luma) >= 6000 {
        return fg;
    }

    if bg_luma > 32768 {
        Color::Black
    } else {
        Color::White
    }
}

fn rgb_of(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        Color::Reset => (128, 128, 128),
        _ => (128, 128, 128),
    }
}

fn truncate_to_width(s: &str, max_cols: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for ch in s.chars() {
        let w = ch.to_string().width();
        if width + w > max_cols {
            break;
        }
        result.push(ch);
        width += w;
    }
    result
}

// ---------------------------------------------------------------------------
// JavaScript — text suppression
// ---------------------------------------------------------------------------

/// Injected once per page load (on `LoadStatus::Complete`) to make all text
/// transparent in Servo's pixel render. Layout and bounding boxes are fully
/// preserved — only the paint color changes — so `EXTRACTION_SCRIPT` still
/// returns accurate positions for the native terminal text overlay.
///
/// A sentinel attribute (`data-carboxyl-suppress`) guards against duplicate
/// injection on pages that fire multiple load-complete notifications.
pub const SUPPRESS_TEXT_SCRIPT: &str = r#"
(function() {
    const ATTR = 'data-carboxyl-suppress';
    if (document.documentElement.hasAttribute(ATTR)) return;
    document.documentElement.setAttribute(ATTR, '1');

    const style = document.createElement('style');
    style.id = 'carboxyl-text-suppress';
    style.textContent = `
        * {
            color: transparent !important;
            caret-color: transparent !important;
            text-shadow: none !important;
        }
    `;
    (document.head || document.documentElement).appendChild(style);
})()
"#;

// ---------------------------------------------------------------------------
// JavaScript — text extraction
// ---------------------------------------------------------------------------

/// Evaluates on the page and returns an Array of Objects with fields:
/// `t` (text), `x`, `y`, `w`, `h` (viewport-relative CSS px), `c` (CSS color).
///
/// Covers:
/// - Regular text nodes via TreeWalker
/// - `<button>` and `<select>` elements
/// - `<input>` and `<textarea>` values
/// - `[contenteditable]` elements
/// - Filters: off-screen, zero-size, hidden, whitespace-only
pub const EXTRACTION_SCRIPT: &str = r#"
(function() {
    const vh = window.innerHeight;
    const vw = window.innerWidth;
    const seen = new WeakSet();
    const nodes = [];

    function visible(r) {
        return r && r.width > 0 && r.height > 0
            && r.bottom > 0 && r.top < vh
            && r.right > 0 && r.left < vw;
    }

    function push(el, text, r) {
        if (seen.has(el)) return;
        seen.add(el);
        const s = getComputedStyle(el);
        if (s.display === 'none' || s.visibility === 'hidden' || s.opacity === '0') return;
        // color is now transparent after injection; read the channel values
        // anyway — they survive the transparency and parse_css_color ignores alpha.
        nodes.push({ t: text, x: r.left, y: r.top, w: r.width, h: r.height, c: s.color });
    }

    // --- Regular text nodes ---
    const walker = document.createTreeWalker(
        document.body || document.documentElement,
        4, // NodeFilter.SHOW_TEXT
        null
    );
    let node;
    while ((node = walker.nextNode())) {
        const text = (node.textContent || '').trim();
        if (!text) continue;
        const el = node.parentElement;
        if (!el) continue;
        const r = el.getBoundingClientRect();
        if (!visible(r)) continue;
        push(el, text, r);
    }

    // --- Form controls + buttons whose text isn't reachable via TreeWalker ---
    const controls = document.querySelectorAll(
        'button, select, ' +
        'input[type="text"], input[type="search"], input[type="submit"], ' +
        'input[type="button"], input[type="reset"], input[type="email"], ' +
        'input[type="url"], input[type="tel"], input[type="number"], ' +
        'input:not([type]), textarea, [contenteditable]'
    );
    for (const el of controls) {
        const text = ((el.value !== undefined && el.value !== '')
            ? el.value
            : el.textContent || '').trim();
        if (!text) continue;
        const r = el.getBoundingClientRect();
        if (!visible(r)) continue;
        push(el, text, r);
    }

    return nodes;
})()
"#;
