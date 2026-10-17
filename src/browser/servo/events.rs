use dpi::PhysicalSize;
use servo::InputEvent;
use url::Url;

use crate::input;
use crate::output::PageFrame;

pub enum RuntimeEvent {
    Input(input::Event),
    /// Servo has something new to show, so the loop should consider painting.
    Wake,
    /// One composited frame: Servo's pixels and the text painted into them.
    Frame(PageFrame),
    Delegate(DelegateEvent),
    /// Terminal was resized to (cols, rows).
    Resize(u16, u16),
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

pub enum ServoCommand {
    Load(Url),
    GoBack,
    GoForward,
    Reload,
    Resize(PhysicalSize<u32>),
    Input(InputEvent),
    /// Run Servo's event loop without requesting a new composite.
    Wake,
    /// Composite, and send back a frame if anything changed.
    Paint,
    Shutdown,
}
