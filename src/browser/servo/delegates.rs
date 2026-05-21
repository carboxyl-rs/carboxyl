use std::sync::mpsc;

use log::{error, warn};
use servo::{
    AuthenticationRequest, LoadStatus, ServoDelegate, ServoError, WebView, WebViewDelegate,
};
use url::Url;

use super::events::{DelegateEvent, RuntimeEvent, ServoCommand};

// ---------------------------------------------------------------------------
// WebView delegate — pure event emitter, owns nothing
// ---------------------------------------------------------------------------

pub struct TerminalWebViewDelegate {
    pub event_tx: mpsc::SyncSender<RuntimeEvent>,
    pub servo_tx: mpsc::SyncSender<ServoCommand>,
    pub native_text: bool,
}

impl WebViewDelegate for TerminalWebViewDelegate {
    fn notify_url_changed(&self, _: WebView, url: Url) {
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::UrlChanged(url)));
    }

    fn notify_page_title_changed(&self, _: WebView, title: Option<String>) {
        let title = title.unwrap_or_else(|| "Carboxyl".to_owned());
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::TitleChanged(title)));
    }

    fn notify_new_frame_ready(&self, _: WebView) {
        let _ = self.event_tx.try_send(RuntimeEvent::Wake);
    }

    fn notify_history_changed(&self, _: WebView, entries: Vec<Url>, current: usize) {
        let Some(url) = entries.get(current) else {
            return;
        };
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::HistoryChanged {
                url: url.clone(),
                can_go_back: current > 0,
                can_go_forward: current + 1 < entries.len(),
            }));
    }

    fn notify_animating_changed(&self, _: WebView, animating: bool) {
        if animating {
            let _ = self.event_tx.try_send(RuntimeEvent::Wake);
        }
    }

    fn notify_closed(&self, _: WebView) {
        let _ = self
            .event_tx
            .try_send(RuntimeEvent::Delegate(DelegateEvent::Closed));
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

    fn notify_load_status_changed(&self, _: WebView, status: LoadStatus) {
        if self.native_text && matches!(status, LoadStatus::HeadParsed) {
            let _ = self.servo_tx.try_send(ServoCommand::SuppressText);
        }

        if self.native_text && matches!(status, LoadStatus::Complete) {
            let _ = self.event_tx.try_send(RuntimeEvent::TextExtractRequested);
        }
    }
}

// ---------------------------------------------------------------------------
// Servo delegate
// ---------------------------------------------------------------------------

pub struct TerminalServoDelegate;

impl ServoDelegate for TerminalServoDelegate {
    fn notify_error(&self, error: ServoError) {
        error!("servo error: {error:?}");
    }
}
