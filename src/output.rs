mod color;
mod compose;
mod graphics;
mod nav_widget;
mod page;
mod raster;
mod window;

pub use compose::{Canvas, Options as ComposeOptions};
pub use graphics::Graphics;
pub use nav_widget::{NavAction, NavState, NavWidget, NavigationCapability};
pub use page::{PageFrame, PxRect, TextBox};
pub use window::{DEFAULT_CELL, NAV_BAR_ROWS, Window, terminal_cell_size};

use ratatui::crossterm::event::{DisableMouseCapture, PopKeyboardEnhancementFlags};
use ratatui::crossterm::execute;
use std::io;

pub fn restore_terminal() {
    execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
    )
    .ok();

    ratatui::restore();
}
