mod browser_widget;
mod nav_widget;
mod text_overlay;
mod window;

pub use browser_widget::{BrowserFrame, BrowserWidget};
pub use nav_widget::{NavAction, NavState, NavWidget};
pub use text_overlay::{EXTRACTION_SCRIPT, TextNode, TextOverlay, parse_js_nodes};
pub use window::Window;
