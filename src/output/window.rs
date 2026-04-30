use glam::{UVec2, Vec2};
use rustix::stdio::stdout;
use rustix::termios::{Winsize, tcgetwinsize};

use crate::cli::Cli;

/// Snapshot of the terminal window dimensions, derived from `TIOCGWINSZ`
/// and the CLI resolution setting. Cheap to clone — passed around by value.
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
            cells: UVec2::new(80, 23),
            cell_pixels: Vec2::new(2.0, 4.0),
            browser: UVec2::ZERO,
        };
        w.update(cli);
        w
    }

    pub fn update(&mut self, cli: &Cli) {
        let Winsize { ws_col, ws_row, .. } = tcgetwinsize(stdout()).unwrap_or(Winsize {
            ws_col: 80,
            ws_row: 24,
            ws_xpixel: 0,
            ws_ypixel: 0,
        });

        let cols = ws_col.max(1) as u32;
        let rows = ws_row.max(2) as u32 - 1;

        let zoom = cli.resolution as f32 / 100.0;
        let scale = Vec2::new(2.0 * zoom, 4.0 * zoom);

        self.cells = UVec2::new(cols, rows);
        self.cell_pixels = scale;
        self.browser = UVec2::new(
            (cols as f32 * scale.x).ceil() as u32,
            (rows as f32 * scale.y).ceil() as u32,
        );
    }

    /// Produce an updated Window for a terminal resize event.
    /// Reuses `cell_pixels` from the current window — only cell count changes.
    pub fn resize(&self, cols: u16, rows: u16) -> Self {
        let cells = UVec2::new(cols as u32, rows.saturating_sub(1) as u32);
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
