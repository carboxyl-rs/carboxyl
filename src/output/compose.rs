//! Compose page pixels and text into native glyphs, solid backgrounds, and
//! quadrant blocks. Detailed regions become tiles for the graphics layer.

use std::cmp::Reverse;

use glam::UVec2;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::color::{Rgb, to_terminal_color};
use super::page::{PageFrame, TextBox};
use super::raster::{Band, Pixels};

/// Smallest region worth handing to a graphics protocol. Anything smaller keeps
/// the quadrant blocks it already has, which avoids a per-icon upload.
const MIN_TILE_CELLS: u32 = 4;

/// Upper bound on graphics tiles per frame. Pages that fragment into more than
/// this keep their largest regions and leave the rest as quadrant blocks.
const MAX_TILES: usize = 64;

/// A cell a native glyph owns, with the ink and background Servo painted there.
/// `None` means the cell still belongs to the pixel layer.
type Painted = Option<(Rgb, Rgb)>;

/// Reusable scratch space for composing frames, so a steady-state repaint does
/// not allocate.
#[derive(Default)]
pub struct Canvas {
    /// Per cell: the colors of the native glyph that owns it, if any.
    glyphs: Vec<Painted>,
    /// Per cell: whether it still wants real pixels.
    detailed: Vec<bool>,
    text: TextScratch,
    tiles: Vec<Rect>,
}

/// The terminal grid one frame is composed onto.
#[derive(Clone, Copy)]
struct Grid {
    /// Where the browser viewport starts in the terminal buffer.
    area: Rect,
    cols: u16,
    rows: u16,
    /// Device pixels per cell. The viewport is an exact multiple of this.
    cell: UVec2,
}

impl Grid {
    fn index(&self, col: u16, row: u16) -> usize {
        usize::from(row) * usize::from(self.cols) + usize::from(col)
    }

    fn position(&self, col: u16, row: u16) -> (u16, u16) {
        (self.area.x + col, self.area.y + row)
    }
}

/// Where one wrapped line of text goes.
#[derive(Clone, Copy)]
struct Slot {
    row: u16,
    col0: u16,
    col1: u16,
}

#[derive(Default)]
struct TextScratch {
    order: Vec<u32>,
    bands: Vec<Band>,
    lines: Vec<String>,
    alternate: Vec<String>,
}

impl Canvas {
    /// Draw `frame` into `area`, returning the regions that want real pixels.
    ///
    /// Tiles are relative to `area` and never overlap a native glyph, so a
    /// graphics protocol can own them outright.
    pub fn compose(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        frame: &PageFrame,
        cell: UVec2,
        options: Options,
    ) -> &[Rect] {
        self.tiles.clear();

        let Some(pixels) = Pixels::new(&frame.pixels, frame.size) else {
            return &self.tiles;
        };
        if cell.x == 0 || cell.y == 0 {
            return &self.tiles;
        }

        let fits = |extent: u32, step: u32| (extent / step).min(u32::from(u16::MAX)) as u16;
        let grid = Grid {
            area,
            cols: area.width.min(fits(frame.size.x, cell.x)),
            rows: area.height.min(fits(frame.size.y, cell.y)),
            cell,
        };
        if grid.cols == 0 || grid.rows == 0 {
            return &self.tiles;
        }

        let cells = usize::from(grid.cols) * usize::from(grid.rows);
        self.glyphs.clear();
        self.glyphs.resize(cells, None);
        self.detailed.clear();
        self.detailed.resize(cells, false);

        if options.native_text {
            place_text(
                &frame.text,
                &pixels,
                &grid,
                buf,
                &mut self.glyphs,
                &mut self.text,
            );
        }

        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let index = grid.index(col, row);
                let Some(target) = buf.cell_mut(grid.position(col, row)) else {
                    continue;
                };
                match self.glyphs[index] {
                    // The glyph itself was written by `place_text`; it only
                    // needs the colors Servo painted into this cell.
                    Some((ink, background)) => {
                        target.set_style(paint(ink, background, options.true_color));
                    }
                    None => {
                        let sample = pixels.cell(col, row, grid.cell);
                        if sample.is_flat() {
                            target.set_symbol(" ").set_style(
                                Style::new()
                                    .bg(to_terminal_color(sample.background, options.true_color)),
                            );
                        } else {
                            let (symbol, ink, background) = sample.quadrant();
                            target.set_char(symbol).set_style(paint(
                                ink,
                                background,
                                options.true_color,
                            ));
                            self.detailed[index] = true;
                        }
                    }
                }
            }
        }

        // A viewport smaller than its area (briefly, during a resize) must not
        // leave the previous frame's cells behind.
        for row in 0..area.height {
            for col in 0..area.width {
                if col < grid.cols && row < grid.rows {
                    continue;
                }
                if let Some(target) = buf.cell_mut(grid.position(col, row)) {
                    target.reset();
                }
            }
        }

        merge_tiles(&grid, &self.detailed, &mut self.tiles);
        &self.tiles
    }
}

/// How a frame should be drawn.
#[derive(Clone, Copy)]
pub struct Options {
    pub true_color: bool,
    pub native_text: bool,
}

fn paint(ink: Rgb, background: Rgb, true_color: bool) -> Style {
    Style::new()
        .fg(to_terminal_color(ink, true_color))
        .bg(to_terminal_color(background, true_color))
}

fn place_text(
    boxes: &[TextBox],
    pixels: &Pixels,
    grid: &Grid,
    buf: &mut Buffer,
    glyphs: &mut [Painted],
    scratch: &mut TextScratch,
) {
    // Deepest boxes first: a link inside a paragraph claims its own cells
    // before the paragraph's text is laid out, and the paragraph then flows
    // around them. That reproduces inline layout without per-line geometry.
    // Taken out of the scratch space so the rest of it stays writable below.
    let mut order = std::mem::take(&mut scratch.order);
    order.clear();
    order.extend(0..boxes.len() as u32);
    order.sort_by_key(|index| Reverse(boxes[*index as usize].depth));

    for &index in &order {
        let text_box = &boxes[index as usize];
        if !text_box.rect.is_visible_in(pixels.size()) {
            continue;
        }

        // Where - and whether - this box's lines actually came out on screen.
        pixels.text_rows(text_box.rect, grid.cell, &mut scratch.bands);
        if scratch.bands.is_empty() {
            continue;
        }

        let col0 = cell_column(text_box.rect.x0, grid.cell.x, grid.cols);
        if col0 >= grid.cols {
            continue;
        }
        // A box narrower than one cell still gets one column to draw in.
        let col1 = cell_column(text_box.rect.x1, grid.cell.x, grid.cols)
            .max(col0 + 1)
            .min(grid.cols);

        let width = usize::from(col1 - col0);
        wrap(&collapse(&text_box.text), width, &mut scratch.lines);
        if text_box.text.contains('\n') {
            preformatted(&text_box.text, width, &mut scratch.alternate);
            // Which reading of the text is right is settled by how many lines
            // Servo actually drew: a code block renders one line per newline, a
            // paragraph one line per wrap.
            if scratch.alternate.len() == scratch.bands.len()
                && scratch.lines.len() != scratch.bands.len()
            {
                std::mem::swap(&mut scratch.lines, &mut scratch.alternate);
            }
        }
        if scratch.lines.is_empty() {
            continue;
        }

        // Bands place the lines exactly - row and starting column both - when
        // there are as many of them as there are wrapped lines. Otherwise the
        // lines are spread evenly down the box, which is where a browser puts
        // them, and each starts at the box's own left edge.
        let aligned = scratch.bands.len() == scratch.lines.len();
        let count = scratch.lines.len() as f32;
        let mut previous = None;
        for (line_index, line) in scratch.lines.iter().enumerate() {
            let (row, line, start) = if aligned {
                let band = scratch.bands[line_index];
                // The band's column already carries any indentation, so the
                // line's own leading space would double it.
                (band.row, line.trim_start(), band.col.clamp(col0, col1 - 1))
            } else {
                let centre =
                    text_box.rect.y0 + (line_index as f32 + 0.5) * text_box.rect.height() / count;
                if centre < 0.0 {
                    continue;
                }
                ((centre / grid.cell.y as f32) as u16, line.as_str(), col0)
            };
            // Lines closer together than a cell row cannot both be drawn;
            // keeping the first beats layering them on top of each other.
            if row >= grid.rows || previous == Some(row) {
                continue;
            }
            previous = Some(row);
            draw_line(
                line,
                Slot {
                    row,
                    col0: start,
                    col1,
                },
                pixels,
                grid,
                buf,
                glyphs,
            );
        }
    }

    scratch.order = order;
}

fn draw_line(
    line: &str,
    slot: Slot,
    pixels: &Pixels,
    grid: &Grid,
    buf: &mut Buffer,
    glyphs: &mut [Painted],
) {
    let mut col = slot.col0;

    // The line's first cell carries its paper and its pen, which is what tells
    // a cell further along the line apart from a cell something else is being
    // drawn into.
    let reference = pixels.cell(slot.col0, slot.row, grid.cell);

    for grapheme in line.graphemes(true) {
        let Ok(width) = u16::try_from(grapheme.width()) else {
            continue;
        };
        if width == 0 {
            continue;
        }

        // Step over cells a deeper box already owns, keeping this line's
        // alignment. A word landing right after one gets a separating column:
        // the whitespace either side of a nested box collapsed into that gap,
        // so the column is blanked rather than left showing glyph pixels.
        let mut stepped = false;
        while col < slot.col1 && glyphs[grid.index(col, slot.row)].is_some() {
            col += 1;
            stepped = true;
        }
        if stepped && grapheme != " " && col < slot.col1 {
            let background = pixels.cell(col, slot.row, grid.cell).background;
            if let Some(target) = buf.cell_mut(grid.position(col, slot.row)) {
                target.set_symbol(" ");
            }
            glyphs[grid.index(col, slot.row)] = Some((background, background));
            col += 1;
        }

        if col >= slot.col1 || col + width > slot.col1 {
            break;
        }
        if (1..width).any(|offset| glyphs[grid.index(col + offset, slot.row)].is_some()) {
            // A wide glyph cannot straddle a cell someone else owns.
            col += 1;
            continue;
        }

        let sample = pixels.cell(col, slot.row, grid.cell);
        if !sample.belongs_with(&reference) {
            // Something else is being drawn here - a fixed header, a dropdown,
            // a modal. Leave the cell to the pixel layer, but keep going so the
            // rest of the line stays aligned.
            col += width;
            continue;
        }

        for offset in 0..width {
            if let Some(target) = buf.cell_mut(grid.position(col + offset, slot.row)) {
                target.set_symbol(if offset == 0 { grapheme } else { "" });
            }
            glyphs[grid.index(col + offset, slot.row)] = Some((sample.ink, sample.background));
        }
        col += width;
    }
}

/// Round a device-pixel x coordinate to a cell column inside the grid.
fn cell_column(x: f32, cell_width: u32, cols: u16) -> u16 {
    let column = (x / cell_width as f32).round();
    if column <= 0.0 {
        0
    } else if column >= f32::from(cols) {
        cols
    } else {
        column as u16
    }
}

/// CSS `white-space: normal` collapsing: every run of whitespace, newlines
/// included, becomes a single space.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Greedy word wrap to `width` columns, measured in terminal columns.
fn wrap(text: &str, width: usize, out: &mut Vec<String>) {
    out.clear();
    let width = width.max(1);
    let mut line = String::new();
    let mut used = 0;

    for word in text.split_whitespace() {
        let word_width = word.width();

        if used > 0 && used + 1 + word_width > width {
            out.push(std::mem::take(&mut line));
            used = 0;
        }

        // A word too long for a line of its own - a URL, or CJK text, which has
        // no spaces to break at - is split instead of overflowing.
        if word_width > width {
            if used > 0 {
                out.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            loop {
                let (head, tail) = split_at_width(rest, width);
                if tail.is_empty() {
                    line.push_str(head);
                    used = head.width();
                    break;
                }
                out.push(head.to_owned());
                rest = tail;
            }
            continue;
        }

        if used > 0 {
            line.push(' ');
            used += 1;
        }
        line.push_str(word);
        used += word_width;
    }

    if !line.is_empty() {
        out.push(line);
    }
}

/// Lay the text out as written: newlines are hard breaks and indentation is
/// kept. Lines too long for the box still wrap, so nothing falls off the edge.
fn preformatted(text: &str, width: usize, out: &mut Vec<String>) {
    out.clear();
    let mut wrapped = Vec::new();

    for raw in text.split('\n') {
        let expanded = raw.replace('\t', "    ");
        let trimmed = expanded.trim_end();
        if trimmed.is_empty() {
            out.push(String::new());
            continue;
        }
        let indent: String = trimmed
            .chars()
            .take_while(|character| character.is_whitespace())
            .collect();
        let room = width.saturating_sub(indent.width()).max(1);
        wrap(trimmed.trim_start(), room, &mut wrapped);
        for line in &wrapped {
            out.push(format!("{indent}{line}"));
        }
    }

    // HTML drops one newline after `<pre>`, and authors leave one before the
    // closing tag; neither is a blank line the page displays.
    while out.first().is_some_and(String::is_empty) {
        out.remove(0);
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
}

/// Split `text` at the last grapheme boundary that keeps it within `width`
/// columns. The head is never empty, so callers always make progress.
fn split_at_width(text: &str, width: usize) -> (&str, &str) {
    let mut used = 0;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let next = used + grapheme.width();
        if next > width && offset > 0 {
            return text.split_at(offset);
        }
        used = next;
    }
    (text, "")
}

/// Merge the cells that still want real pixels into as few rectangles as
/// possible, so the graphics layer makes one upload per region rather than one
/// per cell. Equal horizontal runs on consecutive rows grow one rectangle.
fn merge_tiles(grid: &Grid, detailed: &[bool], out: &mut Vec<Rect>) {
    out.clear();
    let mut previous: Vec<((u16, u16), usize)> = Vec::new();
    let mut current: Vec<((u16, u16), usize)> = Vec::new();

    for row in 0..grid.rows {
        current.clear();
        let filled = |col: u16| detailed.get(grid.index(col, row)).copied().unwrap_or(false);
        let mut col = 0;
        while col < grid.cols {
            if !filled(col) {
                col += 1;
                continue;
            }
            let start = col;
            while col < grid.cols && filled(col) {
                col += 1;
            }
            let run = (start, col - start);
            let index = match previous.iter().find(|(seen, _)| *seen == run) {
                Some((_, index)) => {
                    out[*index].height += 1;
                    *index
                }
                None => {
                    out.push(Rect::new(start, row, run.1, 1));
                    out.len() - 1
                }
            };
            current.push((run, index));
        }
        std::mem::swap(&mut previous, &mut current);
    }

    let area = |tile: &Rect| u32::from(tile.width) * u32::from(tile.height);
    out.retain(|tile| area(tile) >= MIN_TILE_CELLS);
    if out.len() > MAX_TILES {
        out.sort_unstable_by_key(|tile| Reverse(area(tile)));
        out.truncate(MAX_TILES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::page::PxRect;

    const CELL: UVec2 = UVec2::new(8, 16);
    const PAPER: Rgb = (255, 255, 255);
    const INK: Rgb = (0, 0, 0);

    /// A page frame built in the units the renderer works in: cells for the
    /// terminal grid, device pixels for everything Servo painted.
    struct Page {
        size: UVec2,
        pixels: Vec<u8>,
        boxes: Vec<TextBox>,
    }

    impl Page {
        fn new(cols: u32, rows: u32) -> Self {
            let size = UVec2::new(cols * CELL.x, rows * CELL.y);
            Self {
                size,
                pixels: [PAPER.0, PAPER.1, PAPER.2, 255].repeat((size.x * size.y) as usize),
                boxes: Vec::new(),
            }
        }

        fn set(&mut self, x: u32, y: u32, (r, g, b): Rgb) {
            if x >= self.size.x || y >= self.size.y {
                return;
            }
            let index = ((y * self.size.x + x) * 4) as usize;
            self.pixels[index..index + 3].copy_from_slice(&[r, g, b]);
        }

        fn fill(&mut self, rect: PxRect, color: Rgb) -> &mut Self {
            for y in rect.y0 as u32..rect.y1 as u32 {
                for x in rect.x0 as u32..rect.x1 as u32 {
                    self.set(x, y, color);
                }
            }
            self
        }

        /// What a line of text looks like to the sampler: a band of sparse ink
        /// a few pixels below the top of its line box.
        fn line(&mut self, rect: PxRect) -> &mut Self {
            let top = rect.y0 as u32 + 4;
            for y in top..top + 6 {
                for x in (rect.x0 as u32..rect.x1 as u32).step_by(3) {
                    self.set(x, y, INK);
                }
            }
            self
        }

        /// Detail no single color describes, so the cell wants real pixels.
        fn detail(&mut self, rect: PxRect) -> &mut Self {
            for y in rect.y0 as u32..rect.y1 as u32 {
                for x in rect.x0 as u32..rect.x1 as u32 {
                    if (x + y) % 2 == 0 {
                        self.set(x, y, INK);
                    }
                }
            }
            self
        }

        fn text(&mut self, rect: PxRect, depth: u16, text: &str) -> &mut Self {
            self.boxes.push(TextBox {
                rect,
                text: text.to_owned(),
                depth,
            });
            self
        }

        fn render(&self, native_text: bool) -> (Buffer, Vec<Rect>) {
            self.render_into(
                Rect::new(
                    0,
                    0,
                    (self.size.x / CELL.x) as u16,
                    (self.size.y / CELL.y) as u16,
                ),
                native_text,
            )
        }

        fn render_into(&self, area: Rect, native_text: bool) -> (Buffer, Vec<Rect>) {
            let mut buffer = Buffer::empty(area);
            let frame = PageFrame {
                size: self.size,
                pixels: self.pixels.clone(),
                text: self.boxes.clone().into(),
            };
            let mut canvas = Canvas::default();
            let tiles = canvas
                .compose(
                    area,
                    &mut buffer,
                    &frame,
                    CELL,
                    Options {
                        true_color: true,
                        native_text,
                    },
                )
                .to_vec();
            (buffer, tiles)
        }
    }

    fn row(buffer: &Buffer, row: u16) -> String {
        (0..buffer.area.width)
            .map(|col| buffer[(buffer.area.x + col, buffer.area.y + row)].symbol())
            .collect()
    }

    #[test]
    fn a_flat_page_becomes_styled_spaces_and_wants_no_pixels() {
        let (buffer, tiles) = Page::new(4, 2).render(true);
        assert_eq!(row(&buffer, 0), "    ");
        assert_eq!(row(&buffer, 1), "    ");
        assert_eq!(
            buffer[(0, 0)].bg,
            to_terminal_color(PAPER, true),
            "a flat cell keeps the page's own background color"
        );
        assert!(tiles.is_empty());
    }

    #[test]
    fn native_glyphs_land_where_the_page_painted_text() {
        let mut page = Page::new(6, 2);
        page.line(PxRect::new(0.0, 0.0, 48.0, 16.0)).text(
            PxRect::new(0.0, 0.0, 48.0, 16.0),
            1,
            "abc",
        );

        let (buffer, _) = page.render(true);
        assert_eq!(&row(&buffer, 0)[..3], "abc");
        assert_eq!(
            buffer[(0, 0)].fg,
            to_terminal_color(INK, true),
            "glyph color comes from the ink Servo painted"
        );

        // Without native text the same frame is pixels all the way down.
        let (pixels_only, _) = page.render(false);
        assert_ne!(&row(&pixels_only, 0)[..3], "abc");
    }

    #[test]
    fn a_covered_box_is_left_to_the_pixels() {
        // The box is in the tree with its text, but something opaque was
        // painted over it, so the cells hold one flat color and no ink.
        let mut page = Page::new(6, 2);
        page.fill(PxRect::new(0.0, 16.0, 48.0, 32.0), (20, 33, 53))
            .text(PxRect::new(0.0, 16.0, 48.0, 32.0), 1, "hidden");

        let (buffer, _) = page.render(true);
        assert_eq!(row(&buffer, 1), "      ");
        assert_eq!(buffer[(0, 1)].bg, to_terminal_color((20, 33, 53), true));
    }

    #[test]
    fn an_enclosing_box_flows_around_a_nested_one() {
        // <div>Fir <span>mid</span> end</div>: the span is deeper, so it takes
        // its own columns first and the div's text flows around them.
        let mut page = Page::new(12, 1);
        page.line(PxRect::new(0.0, 0.0, 88.0, 16.0))
            .text(PxRect::new(0.0, 0.0, 88.0, 16.0), 2, "Fir  end")
            .text(PxRect::new(32.0, 0.0, 56.0, 16.0), 4, "mid");

        let (buffer, _) = page.render(true);
        assert_eq!(row(&buffer, 0), "Fir mid end ");
    }
}
