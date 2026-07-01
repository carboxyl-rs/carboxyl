use glam::UVec2;
use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};

use super::color::{LUMA_B, LUMA_G, LUMA_R, LUMA_SHIFT, to_terminal_color};

/// A single rendered frame from Servo's software rendering context.
/// Pixel data is RGBA8888, dimensions match `Window::browser`.
#[derive(Clone)]
pub struct BrowserFrame {
    pub pixels: Vec<u8>,
    pub size: UVec2,
}

/// Ratatui widget that maps a `BrowserFrame` into terminal cells using
/// quadrant block characters for maximum sub-cell resolution.
pub struct BrowserWidget<'a> {
    frame: &'a BrowserFrame,
    true_color: bool,
}

impl<'a> BrowserWidget<'a> {
    pub fn new(frame: &'a BrowserFrame, true_color: bool) -> Self {
        Self { frame, true_color }
    }
}

// ---------------------------------------------------------------------------
// Sub-cell grid constants
// ---------------------------------------------------------------------------

/// Horizontal sub-pixels per terminal cell.
/// Each cell maps to a 2-wide virtual grid for left/right quadrant sampling.
const SUBCELL_COLS: usize = 2;

/// Vertical sub-pixels per terminal cell.
/// Each cell maps to a 4-tall virtual grid, split into a top pair (rows 0-1)
/// and a bottom pair (rows 2-3) for quadrant block character selection.
const SUBCELL_ROWS: usize = 4;

/// Row offset of the first bottom-half sub-pixel within a cell.
/// Equals `SUBCELL_ROWS / 2`; the bottom pair of sample points starts here.
const SUBCELL_BOT_ROW: usize = SUBCELL_ROWS / 2;

/// Bytes per pixel in the RGBA8888 frame buffer produced by Servo.
const RGBA_BYTES: usize = 4;

// Compile-time sanity checks.
const _SUBCELL_CHECK: () = assert!(SUBCELL_BOT_ROW * 2 == SUBCELL_ROWS);
const _RGBA_CHECK: () = assert!(RGBA_BYTES == 4);

// ---------------------------------------------------------------------------
// Quadrant block characters
// ---------------------------------------------------------------------------

/// Maps a 4-bit corner mask to a Unicode quadrant block character.
/// Bit layout: bit 3 = top-left, bit 2 = top-right,
///             bit 1 = bottom-left, bit 0 = bottom-right.
/// A set bit means that corner belongs to the foreground (brighter) color.
const QUADRANT: [char; 16] = [
    ' ', '▗', '▖', '▄', '▝', '▐', '▞', '▟', '▘', '▚', '▌', '▙', '▀', '▜', '▛', '█',
];

impl Widget for BrowserWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let fw = self.frame.size.x as usize;
        let fh = self.frame.size.y as usize;

        if fw == 0 || fh == 0 || area.width == 0 || area.height == 0 {
            return;
        }

        // Virtual pixel grid: each terminal cell = SUBCELL_COLS × SUBCELL_ROWS sub-pixels.
        let tw = area.width as usize * SUBCELL_COLS;
        let th = area.height as usize * SUBCELL_ROWS;

        for cy in 0..area.height as usize {
            for cx in 0..area.width as usize {
                // Sample the four corners of this cell's sub-pixel quad.
                let tl = sample(
                    &self.frame.pixels,
                    fw,
                    fh,
                    tw,
                    th,
                    cx * SUBCELL_COLS,
                    cy * SUBCELL_ROWS,
                );
                let tr = sample(
                    &self.frame.pixels,
                    fw,
                    fh,
                    tw,
                    th,
                    cx * SUBCELL_COLS + 1,
                    cy * SUBCELL_ROWS,
                );
                let bl = sample(
                    &self.frame.pixels,
                    fw,
                    fh,
                    tw,
                    th,
                    cx * SUBCELL_COLS,
                    cy * SUBCELL_ROWS + SUBCELL_BOT_ROW,
                );
                let br = sample(
                    &self.frame.pixels,
                    fw,
                    fh,
                    tw,
                    th,
                    cx * SUBCELL_COLS + 1,
                    cy * SUBCELL_ROWS + SUBCELL_BOT_ROW,
                );

                let (fg_rgb, bg_rgb) = fg_bg(tl, tr, bl, br);
                let ch = quadrant_char(tl, tr, bl, br);

                let x = area.x + cx as u16;
                let y = area.y + cy as u16;

                if x < buf.area.right() && y < buf.area.bottom() {
                    buf.cell_mut((x, y)).unwrap().set_char(ch).set_style(
                        Style::new()
                            .fg(to_terminal_color(fg_rgb, self.true_color))
                            .bg(to_terminal_color(bg_rgb, self.true_color)),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pixel helpers
// ---------------------------------------------------------------------------

type Rgb = (u8, u8, u8);

/// Sample one pixel from the frame buffer using nearest-neighbour scaling.
/// Frame is RGBA8888; alpha is discarded.
fn sample(pixels: &[u8], fw: usize, fh: usize, tw: usize, th: usize, tx: usize, ty: usize) -> Rgb {
    let sx = ((tx as f32 + 0.5) * fw as f32 / tw as f32) as usize;
    let sy = ((ty as f32 + 0.5) * fh as f32 / th as f32) as usize;
    let x = sx.min(fw - 1);
    let y = sy.min(fh - 1);
    let i = (y * fw + x) * RGBA_BYTES;
    (pixels[i], pixels[i + 1], pixels[i + 2])
}

fn avg(a: Rgb, b: Rgb) -> Rgb {
    (
        ((a.0 as u16 + b.0 as u16) / 2) as u8,
        ((a.1 as u16 + b.1 as u16) / 2) as u8,
        ((a.2 as u16 + b.2 as u16) / 2) as u8,
    )
}

/// BT.601 luma in the 0..=255 range, used for corner brightness comparisons.
fn luma((r, g, b): Rgb) -> u16 {
    ((r as u32 * LUMA_R + g as u32 * LUMA_G + b as u32 * LUMA_B) >> LUMA_SHIFT) as u16
}

/// Derive fg (brighter pair average) and bg (darker pair average) colors
/// consistent with `quadrant_char`'s bit assignment.
fn fg_bg(tl: Rgb, tr: Rgb, bl: Rgb, br: Rgb) -> (Rgb, Rgb) {
    let mut corners = [
        (luma(tl), tl),
        (luma(tr), tr),
        (luma(bl), bl),
        (luma(br), br),
    ];
    corners.sort_unstable_by_key(|(l, _)| *l);
    let bg = avg(corners[0].1, corners[1].1);
    let fg = avg(corners[2].1, corners[3].1);
    (fg, bg)
}

/// Choose the quadrant block character based on which corners are above
/// the midpoint between the minimum and maximum luma values.
fn quadrant_char(tl: Rgb, tr: Rgb, bl: Rgb, br: Rgb) -> char {
    let lumas = [luma(tl), luma(tr), luma(bl), luma(br)];
    let lo = *lumas.iter().min().unwrap();
    let hi = *lumas.iter().max().unwrap();
    let mid = (lo + hi) / 2;

    let idx = ((lumas[0] > mid) as usize) << 3
        | ((lumas[1] > mid) as usize) << 2
        | ((lumas[2] > mid) as usize) << 1
        | ((lumas[3] > mid) as usize);

    QUADRANT[idx]
}
