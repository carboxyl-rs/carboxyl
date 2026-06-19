mod browser_widget;
pub(crate) mod color;
#[cfg(feature = "native-text")]
mod native_text;
mod nav_widget;
mod window;

pub use browser_widget::{BrowserFrame, BrowserWidget};
pub use nav_widget::{NavAction, NavState, NavWidget, NavigationCapability};
#[cfg(feature = "native-text")]
pub use native_text::{TextNode, TextOverlay};
pub use window::Window;

use crossterm::event::{DisableMouseCapture, PopKeyboardEnhancementFlags};
use std::io;

pub fn restore_terminal() {
    crossterm::execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
    )
    .ok();

    ratatui::restore();
}
