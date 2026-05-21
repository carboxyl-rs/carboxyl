use glam::{UVec2, Vec2};
use rustix::stdio::stdout;
use rustix::termios::{Winsize, tcgetwinsize};

use crate::cli::Cli;

// ---------------------------------------------------------------------------
// Fallback terminal dimensions (used when TIOCGWINSZ fails)
// ---------------------------------------------------------------------------

const FALLBACK_COLS: u16 = 80;
const FALLBACK_ROWS: u16 = 24;

// Minimum meaningful dimensions: at least 1 column, 2 rows (1 nav + 1 browser).
const MIN_COLS: u16 = 1;
const MIN_ROWS: u16 = 2;

// ---------------------------------------------------------------------------
// Default cell-pixel mapping (used before the first TIOCGWINSZ measurement)
// ---------------------------------------------------------------------------

/// Approximate pixels per cell column in the default configuration.
const DEFAULT_CELL_PX_X: f32 = 2.0;
/// Approximate pixels per cell row in the default configuration.
const DEFAULT_CELL_PX_Y: f32 = 4.0;

// ---------------------------------------------------------------------------
// Scale / zoom constants
// ---------------------------------------------------------------------------

/// Denominator for the CLI `--scale` percentage: `scale / SCALE_DIVISOR` gives
/// the zoom multiplier. 100 means 100% = 1.0× zoom.
const SCALE_DIVISOR: f32 = 100.0;

/// Minimum legal value for `--scale`. Guards the division in `zoom`.
const MIN_SCALE: u16 = 1;

/// Baseline pixels-per-cell used as the numerator in the zoom calculation.
/// Dividing by zoom (rather than multiplying) means higher scale values
/// render larger content into a proportionally smaller viewport.
///
/// Calibrated so that `scale = 100` reproduces the old hard-coded 325% zoom.
const BASE_CELL_PX: Vec2 = Vec2::new(6.5, 13.0);

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Number of rows reserved for the navigation bar; subtracted from the
/// terminal row count before computing the browser viewport height.
const NAV_BAR_ROWS: u16 = 1;

/// Snapshot of the terminal window dimensions, derived from `TIOCGWINSZ`
/// and the CLI scale setting. Cheap to clone — passed around by value.
#[derive(Clone, Debug)]
pub struct Window {
    /// Terminal size in cells (nav bar row excluded).
    pub cells: UVec2,
    /// Pixels per terminal cell in the Servo viewport.
    pub cell_pixels: Vec2,
    /// Servo browser viewport in physical pixels.
    pub browser: UVec2,
}

impl Window {
    pub fn read(cli: &Cli) -> Self {
        let mut w = Self {
            cells: UVec2::new(FALLBACK_COLS as u32, (FALLBACK_ROWS - NAV_BAR_ROWS) as u32),
            cell_pixels: Vec2::new(DEFAULT_CELL_PX_X, DEFAULT_CELL_PX_Y),
            browser: UVec2::ZERO,
        };
        w.update(cli);
        w
    }

    pub fn update(&mut self, cli: &Cli) {
        let Winsize { ws_col, ws_row, .. } = tcgetwinsize(stdout()).unwrap_or(Winsize {
            ws_col: FALLBACK_COLS,
            ws_row: FALLBACK_ROWS,
            ws_xpixel: 0,
            ws_ypixel: 0,
        });

        let cols = ws_col.max(MIN_COLS) as u32;
        let rows = ws_row.max(MIN_ROWS) as u32 - NAV_BAR_ROWS as u32;

        // Browser-style zoom semantics:
        //
        // Higher zoom percentage => larger content => smaller viewport.
        //
        // 100% is calibrated to roughly the old 325%.
        let zoom = (cli.scale.max(MIN_SCALE) as f32) / SCALE_DIVISOR;

        // Divide instead of multiply so increasing zoom enlarges content.
        let scale = BASE_CELL_PX / zoom;

        self.cells = UVec2::new(cols, rows);
        self.cell_pixels = scale;
        self.browser = UVec2::new(
            (cols as f32 * scale.x).ceil() as u32,
            (rows as f32 * scale.y).ceil() as u32,
        );
    }

    /// Produce an updated `Window` for a terminal resize event.
    /// Reuses `cell_pixels` from the current window — only cell count changes.
    pub fn resize(&self, cols: u16, rows: u16) -> Self {
        let cells = UVec2::new(
            cols as u32,
            rows.saturating_sub(NAV_BAR_ROWS) as u32,
        );
        let browser = UVec2::new(
            (cells.x as f32 * self.cell_pixels.x).ceil() as u32,
            (cells.y as f32 * self.cell_pixels.y).ceil() as u32,
        );
        Self {
            cells,
            browser,
            cell_pixels: self.cell_pixels,
        }
    }

    pub fn differs_from(&self, other: &Window) -> bool {
        self.cells != other.cells || self.browser != other.browser
    }
}
