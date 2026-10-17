//! Sample page pixels for background, ink, and quadrant colors.
//! Ink bands guide text placement and detect covered text.

use glam::UVec2;

use super::color::{Rgb, luma};
use super::page::PxRect;

/// Luma buckets used to find a region's dominant color. Sixteen separates ink
/// from background while keeping the histogram small enough to stay hot.
const LUMA_BUCKETS: usize = 16;

/// Below this luma spread a cell is treated as one flat color, so it is drawn
/// as a styled space instead of a quadrant approximation.
const FLAT_CONTRAST: u32 = 12;

/// Luma distance from a box's background at which a pixel counts as ink.
const INK_CONTRAST: u32 = 16;

/// Per-channel difference under which two colors count as the same paint.
const SAME_PAINT: u8 = 16;

/// Blank pixel rows tolerated inside one band, so the two parts of a glyph
/// (the dot on an `i`, say) are not counted as two lines.
const BAND_GAP: u32 = 1;

/// Shortest band accepted as a line of text. Underlines and hairline rules are
/// one or two pixels tall; glyph bands are taller.
const MIN_BAND_HEIGHT: u32 = 3;

/// Fraction of a box's width trimmed from each side before looking for ink, so
/// a left border or a bullet does not mark every row as inked.
const EDGE_TRIM_DIVISOR: u32 = 8;
/// Upper bound on that trim, in pixels.
const EDGE_TRIM_MAX: u32 = 4;

/// Maps a 4-bit corner mask to a Unicode quadrant block.
/// Bit 3 = top-left, bit 2 = top-right, bit 1 = bottom-left, bit 0 = bottom-right.
const QUADRANT: [char; 16] = [
    ' ', '▗', '▖', '▄', '▝', '▐', '▞', '▟', '▘', '▚', '▌', '▙', '▀', '▜', '▛', '█',
];

/// Bytes per pixel in Servo's RGBA8888 output.
const RGBA: usize = 4;

/// A borrowed, bounds-checked view of one RGBA8888 frame.
#[derive(Clone, Copy)]
pub struct Pixels<'a> {
    data: &'a [u8],
    size: UVec2,
}

impl<'a> Pixels<'a> {
    /// Returns `None` unless `data` really holds a `size` frame.
    pub fn new(data: &'a [u8], size: UVec2) -> Option<Self> {
        let needed = (size.x as usize)
            .checked_mul(size.y as usize)?
            .checked_mul(RGBA)?;
        (size.x > 0 && size.y > 0 && data.len() >= needed).then_some(Self { data, size })
    }

    pub fn size(&self) -> UVec2 {
        self.size
    }

    fn rgb(&self, x: u32, y: u32) -> Rgb {
        let x = x.min(self.size.x - 1) as usize;
        let y = y.min(self.size.y - 1) as usize;
        let index = (y * self.size.x as usize + x) * RGBA;
        (self.data[index], self.data[index + 1], self.data[index + 2])
    }

    /// Describe the cell at `(col, row)` in one pass over its pixels.
    pub fn cell(&self, col: u16, row: u16, cell: UVec2) -> Cell {
        let origin = UVec2::new(u32::from(col) * cell.x, u32::from(row) * cell.y);
        let split = (cell / 2).max(UVec2::ONE);

        let mut histogram = Histogram::default();
        let mut quad_count = [0u32; 4];
        let mut quad_sum = [[0u32; 3]; 4];

        for dy in 0..cell.y {
            for dx in 0..cell.x {
                let rgb = self.rgb(origin.x + dx, origin.y + dy);
                histogram.push(rgb);

                let quad = usize::from(dy >= split.y) * 2 + usize::from(dx >= split.x);
                quad_count[quad] += 1;
                add(&mut quad_sum[quad], rgb);
            }
        }

        let (background, ink) = histogram.background_and_ink();
        let mut quadrants = [background; 4];
        for (quadrant, (sum, count)) in quad_sum.into_iter().zip(quad_count).enumerate() {
            quadrants[quadrant] = mean(sum, count);
        }

        Cell {
            background,
            ink,
            contrast: luma(ink).abs_diff(luma(background)),
            quadrants,
        }
    }

    /// The lines of text visible inside `rect`: one [`Band`] per line, with the
    /// cell it starts on.
    ///
    /// Servo has already painted the page, so the pixels inside a text box say
    /// where - and whether - its lines actually came out. An empty result means
    /// the box is not visible: covered by something opaque, fully transparent,
    /// or painted in its own background color. That one signal stands in for
    /// stacking order, which the accessibility tree does not expose.
    ///
    /// The starting column comes from the leftmost ink on each line, so padding,
    /// indentation and `text-align` are picked up from the render rather than
    /// guessed: the accessibility tree reports border boxes, which say nothing
    /// about where inside them the text sits.
    ///
    /// Bands shorter than [`MIN_BAND_HEIGHT`] are dropped so underlines and
    /// hairline rules are not mistaken for lines of text.
    pub fn text_rows(&self, rect: PxRect, cell: UVec2, out: &mut Vec<Band>) {
        out.clear();

        let x0 = rect.x0.max(0.0) as u32;
        let y0 = rect.y0.max(0.0) as u32;
        let x1 = (rect.x1.clamp(0.0, self.size.x as f32).ceil() as u32).min(self.size.x);
        let y1 = (rect.y1.clamp(0.0, self.size.y as f32).ceil() as u32).min(self.size.y);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let trim = ((x1 - x0) / EDGE_TRIM_DIVISOR).min(EDGE_TRIM_MAX);
        let (scan_x0, scan_x1) = if x1 - x0 > trim * 2 {
            (x0 + trim, x1 - trim)
        } else {
            (x0, x1)
        };

        let background = luma(self.dominant_color(scan_x0, y0, scan_x1, y1));

        // Grow one band while ink keeps appearing; a gap wider than BAND_GAP
        // closes it and opens the next. `ink` is the band's leftmost ink.
        let mut band: Option<Open> = None;
        for y in y0..y1 {
            let Some(ink) = (scan_x0..scan_x1)
                .find(|&x| luma(self.rgb(x, y)).abs_diff(background) > INK_CONTRAST)
            else {
                continue;
            };
            band = Some(match band {
                Some(open) if y > open.end + BAND_GAP => {
                    push_band(open, cell, out);
                    Open {
                        start: y,
                        end: y + 1,
                        ink,
                    }
                }
                Some(open) => Open {
                    end: y + 1,
                    ink: open.ink.min(ink),
                    ..open
                },
                None => Open {
                    start: y,
                    end: y + 1,
                    ink,
                },
            });
        }
        if let Some(open) = band {
            push_band(open, cell, out);
        }
    }

    /// The dominant color of a pixel region, sampled on a coarse grid.
    fn dominant_color(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> Rgb {
        let mut histogram = Histogram::default();
        for y in (y0..y1).step_by(2) {
            for x in (x0..x1).step_by(2) {
                histogram.push(self.rgb(x, y));
            }
        }
        histogram.background_and_ink().0
    }
}

/// What Servo painted into one terminal cell.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// The cell's dominant color: its background.
    pub background: Rgb,
    /// The most contrasting color in the cell: its ink.
    pub ink: Rgb,
    /// Luma distance between [`Self::ink`] and [`Self::background`].
    pub contrast: u32,
    quadrants: [Rgb; 4],
}

impl Cell {
    /// True when one solid color describes the cell, so it needs no pixels.
    pub fn is_flat(&self) -> bool {
        self.contrast < FLAT_CONTRAST
    }

    /// True when this cell still shows what `reference` shows.
    ///
    /// `reference` is a cell a line of text is known to occupy, so it carries
    /// that line's two colors: its paper and its pen. A cell whose dominant
    /// color is one of those is part of the same line - blank paper after the
    /// last word, a lightly inked cell, or a heavily inked one where the pen
    /// dominates. A cell whose dominant color is neither belongs to something
    /// else that is being drawn there instead, and must not be written over.
    ///
    /// That is how occlusion is decided for the part of a box that something
    /// opaque covers: a fixed header, a dropdown, a modal. The accessibility
    /// tree exposes no stacking order, so the render is the only witness.
    pub fn belongs_with(&self, reference: &Cell) -> bool {
        near(self.background, reference.background) || near(self.background, reference.ink)
    }

    /// The quadrant block that best approximates the cell, with its colors.
    pub fn quadrant(&self) -> (char, Rgb, Rgb) {
        let mut order = [0, 1, 2, 3];
        order.sort_unstable_by_key(|&quadrant| luma(self.quadrants[quadrant]));
        let background = blend(self.quadrants[order[0]], self.quadrants[order[1]]);
        let foreground = blend(self.quadrants[order[2]], self.quadrants[order[3]]);

        let midpoint = (luma(self.quadrants[order[0]]) + luma(self.quadrants[order[3]])) / 2;
        let bit = |quadrant: usize| u8::from(luma(self.quadrants[quadrant]) > midpoint);
        let mask = (bit(0) << 3) | (bit(1) << 2) | (bit(2) << 1) | bit(3);

        (QUADRANT[usize::from(mask)], foreground, background)
    }
}

/// One line of text found inside a box, in terminal-grid coordinates: the row
/// it sits on and the cell its leftmost ink falls in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Band {
    pub row: u16,
    pub col: u16,
}

/// A luma histogram that also remembers each bucket's average color, so the
/// dominant and most-contrasting colors fall out of one pass over the pixels.
#[derive(Default)]
struct Histogram {
    count: [u32; LUMA_BUCKETS],
    sum: [[u32; 3]; LUMA_BUCKETS],
}

impl Histogram {
    fn push(&mut self, rgb: Rgb) {
        let bucket = (luma(rgb) as usize * LUMA_BUCKETS / 256).min(LUMA_BUCKETS - 1);
        self.count[bucket] += 1;
        add(&mut self.sum[bucket], rgb);
    }

    /// The most populated bucket's color, and the populated bucket furthest
    /// from it in brightness.
    fn background_and_ink(&self) -> (Rgb, Rgb) {
        let dominant = self
            .count
            .iter()
            .enumerate()
            .max_by_key(|(_, count)| **count)
            .map(|(bucket, _)| bucket)
            .unwrap_or(0);
        let inked = (0..LUMA_BUCKETS)
            .filter(|bucket| self.count[*bucket] > 0)
            .max_by_key(|bucket| bucket.abs_diff(dominant))
            .unwrap_or(dominant);
        (
            mean(self.sum[dominant], self.count[dominant]),
            mean(self.sum[inked], self.count[inked]),
        )
    }
}

/// A band of ink being grown, in device pixels.
#[derive(Clone, Copy)]
struct Open {
    start: u32,
    /// Exclusive end: one past the last inked row.
    end: u32,
    /// Leftmost inked pixel seen in the band.
    ink: u32,
}

/// Record a band of inked pixel rows as the cell it starts on, dropping bands
/// too short to be text and bands that collapse onto a row already taken.
fn push_band(band: Open, cell: UVec2, out: &mut Vec<Band>) {
    if band.end - band.start < MIN_BAND_HEIGHT {
        return;
    }
    let row = ((band.start + band.end) / 2 / cell.y.max(1)) as u16;
    if out.last().is_some_and(|last| last.row == row) {
        return;
    }
    out.push(Band {
        row,
        col: (band.ink / cell.x.max(1)) as u16,
    });
}

fn add(sum: &mut [u32; 3], (r, g, b): Rgb) {
    sum[0] += u32::from(r);
    sum[1] += u32::from(g);
    sum[2] += u32::from(b);
}

fn mean(sum: [u32; 3], count: u32) -> Rgb {
    if count == 0 {
        return (0, 0, 0);
    }
    (
        (sum[0] / count) as u8,
        (sum[1] / count) as u8,
        (sum[2] / count) as u8,
    )
}

/// Whether two colors are close enough to be the same paint. Tight enough to
/// separate one element's background from another's, loose enough to let a
/// gentle gradient read as one surface.
fn near(a: Rgb, b: Rgb) -> bool {
    a.0.abs_diff(b.0)
        .max(a.1.abs_diff(b.1))
        .max(a.2.abs_diff(b.2))
        <= SAME_PAINT
}

fn blend(a: Rgb, b: Rgb) -> Rgb {
    (
        ((u16::from(a.0) + u16::from(b.0)) / 2) as u8,
        ((u16::from(a.1) + u16::from(b.1)) / 2) as u8,
        ((u16::from(a.2) + u16::from(b.2)) / 2) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: UVec2 = UVec2::new(2, 2);

    /// A `width` x `height` opaque frame painted by `paint`.
    fn frame(width: u32, height: u32, paint: impl Fn(u32, u32) -> Rgb) -> Vec<u8> {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = paint(x, y);
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        data
    }

    #[test]
    fn rejects_buffers_that_do_not_match_their_size() {
        assert!(Pixels::new(&[0; 16], CELL).is_some());
        assert!(Pixels::new(&[0; 15], CELL).is_none());
        assert!(Pixels::new(&[], UVec2::ZERO).is_none());
    }

    #[test]
    fn a_solid_region_is_flat_and_keeps_its_color() {
        let data = frame(2, 2, |_, _| (10, 20, 30));
        let cell = Pixels::new(&data, CELL).unwrap().cell(0, 0, CELL);
        assert!(cell.is_flat());
        assert_eq!(cell.background, (10, 20, 30));
        assert_eq!(cell.ink, (10, 20, 30));
    }

    #[test]
    fn ink_is_the_minority_color_and_background_the_majority() {
        // Three dark pixels and one bright one: dark is the background.
        let data = frame(2, 2, |x, y| {
            if x == 1 && y == 1 {
                (250, 250, 250)
            } else {
                (0, 0, 0)
            }
        });
        let cell = Pixels::new(&data, CELL).unwrap().cell(0, 0, CELL);
        assert_eq!(cell.background, (0, 0, 0));
        assert_eq!(cell.ink, (250, 250, 250));
        assert!(!cell.is_flat());
    }

    #[test]
    fn a_cell_belongs_with_a_line_when_it_shows_the_same_paint() {
        let cell_of = |paint: &dyn Fn(u32, u32) -> Rgb| {
            let data = frame(2, 2, paint);
            Pixels::new(&data, CELL).unwrap().cell(0, 0, CELL)
        };

        // A line of dark text on white paper.
        let line = cell_of(&|x, y| {
            if x == 1 && y == 1 {
                (10, 10, 10)
            } else {
                (250, 250, 250)
            }
        });

        // Blank paper after the last word is part of the same line.
        assert!(cell_of(&|_, _| (250, 250, 250)).belongs_with(&line));
        // So is a cell the text fills, where the pen is what dominates.
        assert!(cell_of(&|_, _| (10, 10, 10)).belongs_with(&line));
        // Another element drawn over the line is not.
        assert!(!cell_of(&|_, _| (20, 33, 53)).belongs_with(&line));
        // A gentle gradient still reads as the same surface.
        assert!(cell_of(&|_, _| (240, 240, 240)).belongs_with(&line));
    }

    #[test]
    fn quadrant_blocks_follow_the_brighter_corners() {
        let data = frame(
            2,
            2,
            |x, _| {
                if x == 0 { (255, 255, 255) } else { (0, 0, 0) }
            },
        );
        let (symbol, foreground, background) = Pixels::new(&data, CELL)
            .unwrap()
            .cell(0, 0, CELL)
            .quadrant();
        assert_eq!(symbol, '▌');
        assert_eq!(foreground, (255, 255, 255));
        assert_eq!(background, (0, 0, 0));
    }

    #[test]
    fn text_rows_finds_one_band_per_line_and_drops_underlines() {
        // A 10x24 box: glyph bands at y 2..6 and y 12..16, plus a one-pixel
        // underline at y 9, all on white. Cells are 8px tall, so the two lines
        // land on rows 0 and 1 and the underline is too short to count.
        let data = frame(10, 24, |_, y| {
            if (2..6).contains(&y) || (12..16).contains(&y) || y == 9 {
                (0, 0, 0)
            } else {
                (255, 255, 255)
            }
        });
        let mut bands = Vec::new();
        Pixels::new(&data, UVec2::new(10, 24)).unwrap().text_rows(
            PxRect::new(0.0, 0.0, 10.0, 24.0),
            UVec2::new(10, 8),
            &mut bands,
        );
        assert_eq!(
            bands,
            vec![Band { row: 0, col: 0 }, Band { row: 1, col: 0 }]
        );
    }

    #[test]
    fn a_covered_box_reports_no_text_rows() {
        let data = frame(8, 16, |_, _| (20, 33, 53));
        let mut bands = Vec::new();
        Pixels::new(&data, UVec2::new(8, 16)).unwrap().text_rows(
            PxRect::new(0.0, 0.0, 8.0, 16.0),
            UVec2::new(8, 16),
            &mut bands,
        );
        assert!(bands.is_empty());
    }

    #[test]
    fn a_left_border_does_not_mark_every_row_as_inked() {
        let data = frame(
            32,
            16,
            |x, _| {
                if x < 4 { (0, 0, 0) } else { (255, 255, 255) }
            },
        );
        let mut bands = Vec::new();
        Pixels::new(&data, UVec2::new(32, 16)).unwrap().text_rows(
            PxRect::new(0.0, 0.0, 32.0, 16.0),
            UVec2::new(8, 16),
            &mut bands,
        );
        assert!(bands.is_empty());
    }

    #[test]
    fn a_line_reports_the_cell_its_ink_starts_in() {
        // A 32x16 box whose text begins 18 pixels in, as padding would put it.
        let data = frame(32, 16, |x, y| {
            if (4..12).contains(&y) && x >= 18 {
                (0, 0, 0)
            } else {
                (255, 255, 255)
            }
        });
        let mut bands = Vec::new();
        Pixels::new(&data, UVec2::new(32, 16)).unwrap().text_rows(
            PxRect::new(0.0, 0.0, 32.0, 16.0),
            UVec2::new(8, 16),
            &mut bands,
        );
        assert_eq!(bands, vec![Band { row: 0, col: 2 }]);
    }
}
