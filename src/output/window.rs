use rustix::termios::{tcgetwinsize, Winsize};
use rustix::stdio::stdout;

use crate::cli::Cli;
use crate::gfx::Size;

/// The current state of the terminal window, derived from `TIOCGWINSZ` and
/// the CLI zoom setting. Cheap to clone and snapshot.
#[derive(Clone, Debug)]
pub struct Window {
    /// Device pixel ratio: how many physical pixels per logical CSS pixel.
    pub dpi: f32,
    /// Size of one terminal cell in physical pixels (width × height).
    pub cell_pixels: Size<f32>,
    /// Terminal dimensions in cells (excludes the nav bar row).
    pub cells: Size<u32>,
    /// Browser viewport in physical pixels — what Servo renders into.
    pub browser: Size<u32>,
}

impl Window {
    pub fn read(cli: &Cli) -> Self {
        let mut w = Self {
            dpi: 1.0,
            cell_pixels: Size::new(8.0, 16.0),
            cells: Size::new(80, 23),
            browser: Size::new(0, 0),
        };
        w.update(cli);
        w
    }

    pub fn update(&mut self, cli: &Cli) {
        let Winsize { ws_col, ws_row, ws_xpixel, ws_ypixel } =
            tcgetwinsize(stdout()).unwrap_or(Winsize {
                ws_col: 80,
                ws_row: 24,
                ws_xpixel: 0,
                ws_ypixel: 0,
            });

        let term_cols = ws_col.max(1) as u32;
        // Reserve one row for the navigation bar.
        let term_rows = ws_row.max(2) as u32 - 1;

        // If the terminal reports pixel dimensions, derive the cell size.
        // Otherwise fall back to the classic 8×16 monospace assumption.
        // Each terminal cell maps to a 2×4 sub-pixel region in the quadrant
        // block encoding. The zoom level scales how many browser pixels each
        // cell represents, giving Servo more detail to render into.
        let zoom = cli.zoom as f32 / 100.0;
        let scale_x = 2.0 * zoom;
        let scale_y = 4.0 * zoom;

        self.dpi = 1.0; // kept for mouse coordinate scaling, not used for viewport sizing
        self.cell_pixels = Size::new(scale_x, scale_y);
        self.cells = Size::new(term_cols, term_rows);
        self.browser = Size::new(
            (term_cols as f32 * scale_x).ceil() as u32,
            (term_rows as f32 * scale_y).ceil() as u32,
        );
    }

    /// Returns `true` if the layout-relevant dimensions have changed.
    pub fn differs_from(&self, other: &Window) -> bool {
        self.cells != other.cells
            || self.browser != other.browser
            || (self.dpi - other.dpi).abs() > f32::EPSILON
    }
}
