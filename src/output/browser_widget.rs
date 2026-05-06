use glam::UVec2;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

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

/// Quadrant block characters encoding which of the four sub-cell corners
/// are "foreground". Bit layout: bit3=TL, bit2=TR, bit1=BL, bit0=BR.
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

        // Virtual pixel grid: each terminal cell = 2 wide × 4 tall sub-pixels.
        let tw = area.width as usize * 2;
        let th = area.height as usize * 4;

        for cy in 0..area.height as usize {
            for cx in 0..area.width as usize {
                let tl = sample(&self.frame.pixels, fw, fh, tw, th, cx * 2, cy * 4);
                let tr = sample(&self.frame.pixels, fw, fh, tw, th, cx * 2 + 1, cy * 4);
                let bl = sample(&self.frame.pixels, fw, fh, tw, th, cx * 2, cy * 4 + 2);
                let br = sample(&self.frame.pixels, fw, fh, tw, th, cx * 2 + 1, cy * 4 + 2);

                let (fg_rgb, bg_rgb) = fg_bg(tl, tr, bl, br);
                let ch = quadrant_char(tl, tr, bl, br);

                let x = area.x + cx as u16;
                let y = area.y + cy as u16;

                if x < buf.area.right() && y < buf.area.bottom() {
                    buf.cell_mut((x, y)).unwrap().set_char(ch).set_style(
                        Style::new()
                            .fg(to_color(fg_rgb, self.true_color))
                            .bg(to_color(bg_rgb, self.true_color)),
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
/// Frame is RGBA8888; we discard alpha.
fn sample(pixels: &[u8], fw: usize, fh: usize, tw: usize, th: usize, tx: usize, ty: usize) -> Rgb {
    let sx = ((tx as f32 + 0.5) * fw as f32 / tw as f32) as usize;
    let sy = ((ty as f32 + 0.5) * fh as f32 / th as f32) as usize;
    let x = sx.min(fw - 1);
    let y = sy.min(fh - 1);
    let i = (y * fw + x) * 4;
    (pixels[i], pixels[i + 1], pixels[i + 2]) // RGBA → RGB
}

fn avg(a: Rgb, b: Rgb) -> Rgb {
    (
        ((a.0 as u16 + b.0 as u16) / 2) as u8,
        ((a.1 as u16 + b.1 as u16) / 2) as u8,
        ((a.2 as u16 + b.2 as u16) / 2) as u8,
    )
}

fn luma((r, g, b): Rgb) -> u16 {
    (r as u16 * 77 + g as u16 * 150 + b as u16 * 29) >> 8
}

/// Derive fg (brighter pair average) and bg (darker pair average) colors
/// consistent with `quadrant_char`'s assignment.
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
/// the midpoint between the min and max luma values.
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

fn to_color((r, g, b): Rgb, true_color: bool) -> Color {
    if true_color {
        Color::Rgb(r, g, b)
    } else {
        let q = |v: u8| (v / 43).min(5);
        Color::Indexed(16 + q(r) * 36 + q(g) * 6 + q(b))
    }
}
