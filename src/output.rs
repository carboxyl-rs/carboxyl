mod browser_widget;
pub(crate) mod color;
mod native_text;
mod nav_widget;
mod window;

pub use browser_widget::{BrowserFrame, BrowserWidget};
pub use native_text::TextOverlay;
pub use nav_widget::{NavAction, NavState, NavWidget, NavigationCapability};
pub use window::Window;

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
