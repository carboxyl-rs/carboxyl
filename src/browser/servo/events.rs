use dpi::PhysicalSize;
use servo::{DisplayList, InputEvent};
use url::Url;

use crate::input;
use crate::output::BrowserFrame;

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
    /// A layout display-list snapshot delivered by Servo's layout engine.
    DisplayList(DisplayList),
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
    Paint,
    Shutdown,
}
