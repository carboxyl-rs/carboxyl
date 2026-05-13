use dpi::PhysicalSize;
use servo::InputEvent;
use url::Url;

use crate::input;
use crate::output::{BrowserFrame, TextNode};

// ---------------------------------------------------------------------------
// Events flowing into the main loop
// ---------------------------------------------------------------------------

pub enum RuntimeEvent {
    Input(input::Event),
    /// Servo needs the loop to consider painting.
    Wake,
    /// A fully composited frame from the Servo thread.
    Frame(BrowserFrame),
    Delegate(DelegateEvent),
    /// Terminal was resized to (cols, rows).
    Resize(u16, u16),
    /// Text nodes extracted from the page via JS.
    TextNodes(Vec<TextNode>),
    /// Fired by the delegate after load-complete; causes an immediate extract
    /// (bypassing the debounce) so native text appears as soon as the page settles.
    TextExtractRequested,
    Exit,
}

pub enum DelegateEvent {
    UrlChanged(Url),
    TitleChanged(String),
    HistoryChanged {
        url: Url,
        can_go_back: bool,
        can_go_forward: bool,
    },
    Closed,
}

// ---------------------------------------------------------------------------
// Commands sent from the main loop to the Servo thread
// ---------------------------------------------------------------------------

pub enum ServoCommand {
    Load(Url),
    GoBack,
    GoForward,
    Reload,
    Resize(PhysicalSize<u32>),
    Input(InputEvent),
    /// Composite and send back a frame if anything changed.
    Paint,
    /// Run the text extraction script and send results back.
    ExtractText,
    /// Inject the text-suppression stylesheet into the current page.
    SuppressText,
    Shutdown,
}
