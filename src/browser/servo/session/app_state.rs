use crate::output::{BrowserFrame, NavState, NavigationCapability, TextNode, Window};

use super::super::{events::DelegateEvent, geometry::BrowserPoint};

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
    pub text_nodes: Vec<TextNode>,
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
            text_nodes: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // State transitions
    // ------------------------------------------------------------------

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
    /// sequence should be emitted — keeping that I/O side-effect out of here.
    pub fn apply_delegate(&mut self, ev: DelegateEvent) -> Option<String> {
        match ev {
            DelegateEvent::UrlChanged(url) => {
                self.nav.push(
                    url,
                    NavigationCapability {
                        back: self.nav.nav.back,
                        forward: self.nav.nav.forward,
                    },
                );
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

    pub fn apply_text_nodes(&mut self, nodes: Vec<TextNode>, native_text: bool) {
        if native_text {
            self.text_nodes = nodes;
            self.mark_dirty();
        }
    }
}
