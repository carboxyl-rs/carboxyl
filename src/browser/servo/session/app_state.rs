use servo::DisplayList;

use crate::output::{BrowserFrame, NavState, NavigationCapability, Window};

use crate::browser::servo::{events::DelegateEvent, geometry::BrowserPoint};

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// All mutable runtime state for the lifetime of a browser session.
pub struct AppState {
    pub running: bool,
    pub pending_paint: bool,
    pub window: Window,
    pub nav: NavState,
    pub pointer: BrowserPoint,
    pub frame: Option<BrowserFrame>,
    /// The most recently delivered display-list snapshot from Servo's layout
    /// engine. Text runs are in document/viewport space; the per-frame
    /// `BrowserFrame::scroll_offset` supplies the live scroll position.
    pub display_list: Option<DisplayList>,
}

impl AppState {
    pub fn new(window: Window) -> Self {
        Self {
            running: true,
            pending_paint: true,
            window,
            nav: NavState::default(),
            pointer: BrowserPoint::default(),
            frame: None,
            display_list: None,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.pending_paint = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn apply_frame(&mut self, frame: BrowserFrame) {
        self.frame = Some(frame);
        self.mark_dirty();
    }

    /// Returns the new `Window` if the viewport actually changed, so the
    /// caller knows whether to forward a resize command to servo.
    pub fn apply_resize(&mut self, cols: u16, rows: u16) -> Option<Window> {
        let next = self.window.resize(cols, rows);
        if next.differs_from(&self.window) {
            self.window = next;
            self.mark_dirty();
            Some(self.window.clone())
        } else {
            None
        }
    }

    /// Mutates nav state. Returns `Some(title)` when the terminal title OSC
    /// sequence should be emitted - keeping that I/O side-effect out of here.
    pub fn apply_delegate(&mut self, ev: DelegateEvent) -> Option<String> {
        match ev {
            DelegateEvent::UrlChanged(url) => {
                let nav = self.nav.nav;
                self.nav.push(url, nav);
            }

            DelegateEvent::HistoryChanged {
                url,
                can_go_back,
                can_go_forward,
            } => {
                self.nav.push(
                    url,
                    NavigationCapability {
                        back: can_go_back,
                        forward: can_go_forward,
                    },
                );
                self.display_list = None;
            }

            DelegateEvent::TitleChanged(title) => {
                self.mark_dirty();
                return Some(title);
            }

            DelegateEvent::Closed => {
                self.stop();
            }
        }

        self.mark_dirty();
        None
    }

    pub fn apply_display_list(&mut self, dl: DisplayList) {
        if self
            .display_list
            .as_ref()
            .is_some_and(|current| dl.epoch < current.epoch)
        {
            return;
        }
        self.display_list = Some(dl);
        self.mark_dirty();
    }
}
