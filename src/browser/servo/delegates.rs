use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use log::{error, warn};
use servo::accesskit::TreeUpdate;
use servo::{AuthenticationRequest, ServoDelegate, ServoError, WebView, WebViewDelegate};
use url::Url;

use super::a11y::Accessibility;
use super::events::{DelegateEvent, RuntimeEvent};

pub struct TerminalWebViewDelegate {
    pub event_tx: mpsc::SyncSender<RuntimeEvent>,
    /// Shared with the compositor; absent until native text is activated.
    pub a11y: Rc<RefCell<Option<Accessibility>>>,
}

impl TerminalWebViewDelegate {
    fn wake(&self) {
        let _ = self.event_tx.try_send(RuntimeEvent::Wake);
    }

    fn emit(&self, event: DelegateEvent) {
        let _ = self.event_tx.try_send(RuntimeEvent::Delegate(event));
    }
}

impl WebViewDelegate for TerminalWebViewDelegate {
    fn notify_url_changed(&self, _: WebView, url: Url) {
        self.emit(DelegateEvent::UrlChanged(url));
    }

    fn notify_page_title_changed(&self, _: WebView, title: Option<String>) {
        let mut title = title.unwrap_or_else(|| "Carboxyl".to_owned());
        title.retain(|character| !character.is_control());
        self.emit(DelegateEvent::TitleChanged(title));
    }

    fn notify_new_frame_ready(&self, _: WebView) {
        self.wake();
    }

    fn notify_history_changed(&self, _: WebView, entries: Vec<Url>, current: usize) {
        let Some(url) = entries.get(current) else {
            return;
        };
        self.emit(DelegateEvent::HistoryChanged {
            url: url.clone(),
            can_go_back: current > 0,
            can_go_forward: current + 1 < entries.len(),
        });
    }

    fn notify_animating_changed(&self, _: WebView, animating: bool) {
        if animating {
            self.wake();
        }
    }

    fn notify_closed(&self, _: WebView) {
        self.emit(DelegateEvent::Closed);
    }

    /// Wake the compositor when text or its geometry changes, including scrolls.
    fn notify_accessibility_tree_update(&self, _: WebView, update: TreeUpdate) {
        if let Some(a11y) = self.a11y.borrow_mut().as_mut() {
            a11y.update(update);
            self.wake();
        }
    }

    fn request_authentication(&self, _: WebView, request: AuthenticationRequest) {
        let scope = if request.for_proxy() {
            "proxy"
        } else {
            "origin"
        };
        warn!(
            "authentication requested for {} ({scope}); no prompt implemented, denying",
            request.url()
        );
    }
}

pub struct TerminalServoDelegate;

impl ServoDelegate for TerminalServoDelegate {
    fn notify_error(&self, error: ServoError) {
        error!("servo error: {error:?}");
    }
}
