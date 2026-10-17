use crate::output::{NavState, NavigationCapability, PageFrame, Window};

use super::events::DelegateEvent;

/// All mutable runtime state for the lifetime of a browser session.
pub struct AppState {
    pub running: bool,
    /// Something changed that the terminal has not been shown yet.
    pub dirty: bool,
    pub window: Window,
    pub nav: NavState,
    pub frame: Option<PageFrame>,
}

impl AppState {
    pub fn new(window: Window) -> Self {
        Self {
            running: true,
            dirty: true,
            window,
            nav: NavState::default(),
            frame: None,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Skip identical frames; keep frames crossing a resize for clipped display.
    pub fn apply_frame(&mut self, frame: PageFrame) {
        if self
            .frame
            .as_ref()
            .is_some_and(|current| current.matches(&frame))
        {
            return;
        }
        self.frame = Some(frame);
        self.mark_dirty();
    }

    /// Returns the new window if the viewport actually changed, so the caller
    /// knows whether to forward a resize to Servo.
    pub fn apply_resize(&mut self, cols: u16, rows: u16) -> Option<Window> {
        let next = self.window.resized(cols, rows);
        if next == self.window {
            return None;
        }
        self.window = next;
        self.mark_dirty();
        Some(self.window)
    }

    /// Apply a delegate event. Returns `Some(title)` when the terminal title
    /// should be set, keeping that side effect out of here.
    pub fn apply_delegate(&mut self, event: DelegateEvent) -> Option<String> {
        match event {
            DelegateEvent::UrlChanged(url) => {
                // Keep the current capability flags: HistoryChanged follows
                // immediately with the authoritative back/forward state.
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
            }

            DelegateEvent::TitleChanged(title) => {
                self.mark_dirty();
                return Some(title);
            }

            DelegateEvent::Closed => self.stop(),
        }

        self.mark_dirty();
        None
    }
}

#[cfg(test)]
mod tests {
    use glam::UVec2;

    use crate::output::DEFAULT_CELL;

    use super::*;

    fn state() -> AppState {
        AppState::new(Window {
            cells: UVec2::new(4, 2),
            cell: DEFAULT_CELL,
        })
    }

    fn frame(size: UVec2, pixel: u8) -> PageFrame {
        PageFrame {
            size,
            pixels: vec![pixel; (size.x * size.y * 4) as usize],
            text: Vec::new().into(),
            images: Vec::new().into(),
        }
    }

    #[test]
    fn a_frame_in_flight_across_a_resize_is_still_shown() {
        let mut app = state();
        app.apply_frame(frame(UVec2::new(100, 100), 1));
        assert!(
            app.frame.is_some(),
            "the renderer clips a wrong-sized frame rather than blanking the page"
        );
    }

    #[test]
    fn an_identical_frame_does_not_cost_a_redraw() {
        let mut app = state();
        app.apply_frame(frame(app.window.viewport(), 1));
        app.dirty = false;

        app.apply_frame(frame(app.window.viewport(), 1));
        assert!(
            !app.dirty,
            "the same pixels and the same text: nothing to do"
        );

        app.apply_frame(frame(app.window.viewport(), 2));
        assert!(app.dirty);
    }

    #[test]
    fn a_resize_is_reported_once_and_keeps_the_page_on_screen() {
        let mut app = state();
        app.apply_frame(frame(app.window.viewport(), 1));

        let resized = app.apply_resize(10, 5).expect("the viewport changed");
        assert_eq!(resized.cells, UVec2::new(10, 4));
        assert!(
            app.frame.is_some(),
            "the last frame stays up until the next"
        );
        assert!(app.apply_resize(10, 5).is_none(), "no change, no resize");
    }
}
