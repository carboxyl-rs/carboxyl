//! Terminal geometry: the viewport is `cells * cell` device pixels.
//! Cell alignment keeps clicks and graphics crops exact; Servo handles page zoom.

use glam::UVec2;
use rustix::stdio::stdout;
use rustix::termios::{Winsize, tcgetwinsize};

/// Rows reserved for the navigation bar at the top of the terminal.
pub const NAV_BAR_ROWS: u16 = 1;

/// Cell size assumed when the terminal will not report one: the 1:2 ratio
/// nearly every terminal font has, at a size where a page's default 16px text
/// comes to roughly one character per cell.
pub const DEFAULT_CELL: UVec2 = UVec2::new(8, 16);

const FALLBACK_COLS: u16 = 80;
const FALLBACK_ROWS: u16 = 24;

/// The browser viewport is never empty, however small the terminal gets.
const MIN_CELLS: UVec2 = UVec2::ONE;

/// Terminal dimensions, in cells and in device pixels per cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Window {
    /// Browser viewport in cells; the navigation bar row is already excluded.
    pub cells: UVec2,
    /// Device pixels per terminal cell.
    pub cell: UVec2,
}

impl Window {
    /// Read the terminal's current size, using `cell` for the cell geometry.
    pub fn read(cell: UVec2) -> Self {
        let Winsize { ws_col, ws_row, .. } = winsize();
        Self {
            cells: browser_cells(ws_col, ws_row),
            cell: cell.max(MIN_CELLS),
        }
    }

    /// The same window after a resize to `cols` x `rows` terminal cells.
    pub fn resized(self, cols: u16, rows: u16) -> Self {
        Self {
            cells: browser_cells(cols, rows),
            ..self
        }
    }

    /// The Servo viewport in device pixels: an exact multiple of [`Self::cell`].
    pub fn viewport(&self) -> UVec2 {
        self.cells * self.cell
    }
}

/// The cell size the terminal reports through `TIOCGWINSZ`, if it reports one.
///
/// Terminals that fill in the pixel fields give the real cell size for free,
/// with no escape-sequence round trip and no risk of eating a keystroke.
pub fn terminal_cell_size() -> Option<UVec2> {
    let Winsize {
        ws_col,
        ws_row,
        ws_xpixel,
        ws_ypixel,
    } = winsize();
    (ws_xpixel > 0 && ws_ypixel > 0 && ws_col > 0 && ws_row > 0)
        .then(|| UVec2::new(u32::from(ws_xpixel / ws_col), u32::from(ws_ypixel / ws_row)))
        .filter(|cell| cell.x > 0 && cell.y > 0)
}

fn winsize() -> Winsize {
    tcgetwinsize(stdout()).unwrap_or(Winsize {
        ws_col: FALLBACK_COLS,
        ws_row: FALLBACK_ROWS,
        ws_xpixel: 0,
        ws_ypixel: 0,
    })
}

fn browser_cells(cols: u16, rows: u16) -> UVec2 {
    UVec2::new(
        u32::from(cols),
        u32::from(rows.saturating_sub(NAV_BAR_ROWS)),
    )
    .max(MIN_CELLS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewport_is_always_a_whole_number_of_cells() {
        let window = Window {
            cells: UVec2::new(80, 23),
            cell: DEFAULT_CELL,
        };
        assert_eq!(window.viewport(), UVec2::new(640, 368));
        assert_eq!(window.viewport() % window.cell, UVec2::ZERO);
    }

    #[test]
    fn a_minimised_terminal_still_has_somewhere_to_render() {
        let window = Window {
            cells: UVec2::new(80, 23),
            cell: DEFAULT_CELL,
        };
        let resized = window.resized(0, 0);
        assert_eq!(resized.cells, UVec2::ONE);
        assert!(resized.viewport().min_element() > 0);

        // One row is the navigation bar, so a two-row terminal has one left.
        assert_eq!(window.resized(10, 2).cells, UVec2::new(10, 1));
        assert_eq!(window.resized(10, 1).cells, UVec2::new(10, 1));
    }
}
