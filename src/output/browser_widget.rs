use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::gfx::{Color as GfxColor, Size};

/// The quadrant Unicode block used to encode 2×2 pixel regions per cell.
/// Each character encodes which of the four quadrants are "foreground":
///
///   TL TR
///   BL BR
///
/// We exploit this to pack two independently-coloured rows of pixels into a
/// single terminal cell by averaging adjacent pixel pairs.
const QUADRANT: [char; 16] = [
    ' ', '▗', '▖', '▄', '▝', '▐', '▞', '▟', '▘', '▚', '▌', '▙', '▀', '▜', '▛', '█',
];

/// A snapshot of the pixel buffer Servo has rendered. Cheap to clone because
/// the pixel data is reference-counted.
#[derive(Clone)]
pub struct BrowserFrame {
    pub pixels: Vec<u8>,
    pub size: Size<u32>,
}

/// Ratatui widget that maps a `BrowserFrame` into terminal cells using
/// quadrant block characters and true-color RGB styling.
pub struct BrowserWidget<'a> {
    frame: &'a BrowserFrame,
    true_color: bool,
}

impl<'a> BrowserWidget<'a> {
    pub fn new(frame: &'a BrowserFrame, true_color: bool) -> Self {
        Self { frame, true_color }
    }
}

impl Widget for BrowserWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let frame_w = self.frame.size.width as usize;
        let frame_h = self.frame.size.height as usize;

        if frame_w == 0 || frame_h == 0 || area.width == 0 || area.height == 0 {
            return;
        }

        // Each terminal cell covers a 2-pixel-wide × 4-pixel-tall region in
        // the quadrant encoding (top-half / bottom-half × left / right).
        // We map `area` cells to frame pixels via nearest-neighbour sampling.
        let target_w = area.width as usize * 2;
        let target_h = area.height as usize * 4;

        for cell_y in 0..area.height as usize {
            for cell_x in 0..area.width as usize {
                // Sample the four sub-pixel positions for this cell.
                let tl = sample(
                    &self.frame.pixels,
                    frame_w,
                    frame_h,
                    target_w,
                    target_h,
                    cell_x * 2,
                    cell_y * 4,
                );
                let tr = sample(
                    &self.frame.pixels,
                    frame_w,
                    frame_h,
                    target_w,
                    target_h,
                    cell_x * 2 + 1,
                    cell_y * 4,
                );
                let bl = sample(
                    &self.frame.pixels,
                    frame_w,
                    frame_h,
                    target_w,
                    target_h,
                    cell_x * 2,
                    cell_y * 4 + 2,
                );
                let br = sample(
                    &self.frame.pixels,
                    frame_w,
                    frame_h,
                    target_w,
                    target_h,
                    cell_x * 2 + 1,
                    cell_y * 4 + 2,
                );

                // Average pairs vertically to get fg (top) and bg (bottom).
                let fg_color = avg(tl, tr);
                let bg_color = avg(bl, br);

                // Pick the quadrant character that best represents the
                // brightness contrast between the four corners.
                let char = quadrant_char(tl, tr, bl, br);

                let x = area.x + cell_x as u16;
                let y = area.y + cell_y as u16;

                if x < buf.area.right() && y < buf.area.bottom() {
                    let cell = buf.cell_mut((x, y)).unwrap();

                    cell.set_char(char);
                    cell.set_style(
                        Style::new()
                            .fg(to_ratatui_color(fg_color, self.true_color))
                            .bg(to_ratatui_color(bg_color, self.true_color)),
                    );
                }
            }
        }
    }
}

/// Sample a single BGRA pixel from the frame buffer, scaling from target
/// coordinates (where target is `target_w × target_h` virtual pixels
/// covering the same area as the frame) back to frame coordinates.
fn sample(
    pixels: &[u8],
    frame_w: usize,
    frame_h: usize,
    target_w: usize,
    target_h: usize,
    tx: usize,
    ty: usize,
) -> GfxColor {
    let sx = ((tx as f32 + 0.5) * frame_w as f32 / target_w as f32) as usize;
    let sy = ((ty as f32 + 0.5) * frame_h as f32 / target_h as f32) as usize;
    let x = sx.min(frame_w - 1);
    let y = sy.min(frame_h - 1);
    let idx = (y * frame_w + x) * 4;

    // Servo's SoftwareRenderingContext outputs BGRA8888.
    GfxColor::new(
        pixels[idx + 2], // R
        pixels[idx + 1], // G
        pixels[idx],     // B
    )
}

/// Average two colours component-wise.
fn avg(a: GfxColor, b: GfxColor) -> GfxColor {
    GfxColor::new(
        ((a.r as u16 + b.r as u16) / 2) as u8,
        ((a.g as u16 + b.g as u16) / 2) as u8,
        ((a.b as u16 + b.b as u16) / 2) as u8,
    )
}

/// Choose the quadrant block character that best encodes the four corner
/// colours. Each quadrant is "foreground" if its luma is closer to the
/// top-pair average than the bottom-pair average.
fn quadrant_char(tl: GfxColor, tr: GfxColor, bl: GfxColor, br: GfxColor) -> char {
    let fg_luma = luma(avg(tl, tr));
    let bg_luma = luma(avg(bl, br));
    let mid = (fg_luma as u16 + bg_luma as u16) / 2;

    let tl_fg = (luma(tl) as u16) >= mid;
    let tr_fg = (luma(tr) as u16) >= mid;
    let bl_fg = (luma(bl) as u16) >= mid;
    let br_fg = (luma(br) as u16) >= mid;

    // Bit layout: TL=3, TR=2, BL=1, BR=0
    let idx =
        (tl_fg as usize) << 3 | (tr_fg as usize) << 2 | (bl_fg as usize) << 1 | (br_fg as usize);

    QUADRANT[idx]
}

fn luma(c: GfxColor) -> u8 {
    // BT.601 integer approximation, fast and accurate enough for ordering.
    ((c.r as u32 * 77 + c.g as u32 * 150 + c.b as u32 * 29) >> 8) as u8
}

fn to_ratatui_color(c: GfxColor, true_color: bool) -> Color {
    if true_color {
        Color::Rgb(c.r, c.g, c.b)
    } else {
        // Quantise to the 216-color web-safe palette (6×6×6 cube) as a
        // reasonable fallback for terminals without true-color support.
        let q = |v: u8| (v / 43).min(5);
        Color::Indexed(16 + q(c.r) * 36 + q(c.g) * 6 + q(c.b))
    }
}
