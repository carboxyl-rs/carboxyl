use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dpi::PhysicalSize;
use log::{error, warn};
use servo::{
    DeviceIntPoint, DeviceIntRect, DeviceIntSize, JavaScriptEvaluationError, Preferences,
    RenderingContext, ServoBuilder, SoftwareRenderingContext, WebView, WebViewBuilder,
    WebViewDelegate,
};
use url::Url;

use crate::output::{BrowserFrame, EXTRACTION_SCRIPT, SUPPRESS_TEXT_SCRIPT, parse_js_nodes};

use super::delegates::{TerminalServoDelegate, TerminalWebViewDelegate};
use super::events::{RuntimeEvent, ServoCommand};
use super::waker::ServoWaker;

// ---------------------------------------------------------------------------
// Timing constants
// ---------------------------------------------------------------------------

/// Brief sleep after each `spin_event_loop` call. Prevents the servo thread
/// from monopolising a core between paint/extract cycles while still allowing
/// Servo's own timers and animation frames to fire promptly.
const SERVO_SPIN_SLEEP: Duration = Duration::from_millis(1);

// ---------------------------------------------------------------------------
// PendingOps — batch-accumulates commands drained from the servo channel
// ---------------------------------------------------------------------------

/// Flags and deferred data accumulated while draining the command queue in
/// one pass before calling `spin_event_loop`.
///
/// Immediate actions (Load, Input, etc.) are dispatched directly in `apply`;
/// deferred actions (paint, extract, suppress) are flagged and executed
/// afterwards in a fixed, deterministic order.
#[derive(Default)]
struct PendingOps {
    shutdown: bool,
    paint: bool,
    extract: bool,
    suppress: bool,
    resize: Option<PhysicalSize<u32>>,
}

impl PendingOps {
    fn apply(&mut self, cmd: ServoCommand, webview: &WebView) {
        match cmd {
            ServoCommand::Shutdown => self.shutdown = true,
            ServoCommand::Load(url) => webview.load(url),
            ServoCommand::GoBack => {
                if webview.can_go_back() {
                    webview.go_back(1);
                }
            }
            ServoCommand::GoForward => {
                if webview.can_go_forward() {
                    webview.go_forward(1);
                }
            }
            ServoCommand::Reload => webview.reload(),
            ServoCommand::Resize(size) => self.resize = Some(size),
            ServoCommand::Input(ev) => {
                webview.notify_input_event(ev);
            }
            ServoCommand::Paint => self.paint = true,
            ServoCommand::ExtractText => self.extract = true,
            ServoCommand::SuppressText => self.suppress = true,
        }
    }
}

// ---------------------------------------------------------------------------
// Thread entry point
// ---------------------------------------------------------------------------

pub fn servo_thread(
    event_tx: mpsc::SyncSender<RuntimeEvent>,
    servo_tx: mpsc::SyncSender<ServoCommand>,
    servo_rx: mpsc::Receiver<ServoCommand>,
    url: Url,
    browser_size: PhysicalSize<u32>,
    native_text: bool,
) {
    let servo = ServoBuilder::default()
        .preferences(browser_preferences(Preferences::default()))
        .event_loop_waker(Box::new(ServoWaker::new(servo_tx.clone())))
        .build();

    servo.set_delegate(Rc::new(TerminalServoDelegate));

    let rendering_context: Rc<dyn RenderingContext> =
        match SoftwareRenderingContext::new(browser_size) {
            Ok(ctx) => Rc::new(ctx),
            Err(e) => {
                error!("failed to create rendering context: {e:?}");
                let _ = event_tx.try_send(RuntimeEvent::Exit);
                return;
            }
        };

    let delegate: Rc<dyn WebViewDelegate> = Rc::new(TerminalWebViewDelegate {
        event_tx: event_tx.clone(),
        servo_tx: servo_tx.clone(),
        native_text,
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate)
        .url(url)
        .build();

    webview.show();
    webview.focus();

    while let Ok(cmd) = servo_rx.recv() {
        // Drain the full queue before spinning, so a burst of commands is
        // processed in one Servo cycle rather than many round-trips.
        let mut ops = PendingOps::default();
        ops.apply(cmd, &webview);
        while let Ok(cmd) = servo_rx.try_recv() {
            ops.apply(cmd, &webview);
        }

        if ops.shutdown {
            break;
        }

        if let Some(size) = ops.resize {
            rendering_context.resize(size);
            webview.resize(size);
            ops.paint = true;
        }

        servo.spin_event_loop();

        // Suppress first so Servo repaints with transparent text before we
        // extract node positions — guarantees the two are always paired.
        if ops.suppress {
            suppress_text(&webview);
        }

        if ops.extract {
            extract_text(&webview, event_tx.clone());
        }

        if ops.paint && let Some(frame) = paint(&webview, rendering_context.as_ref()) {
            let _ = event_tx.try_send(RuntimeEvent::Frame(frame));
        }

        thread::sleep(SERVO_SPIN_SLEEP);
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn suppress_text(webview: &WebView) {
    webview.evaluate_javascript(SUPPRESS_TEXT_SCRIPT, |result| {
        if let Err(e) = result
            && !matches!(e, JavaScriptEvaluationError::WebViewNotReady)
        {
            warn!("text suppression failed: {e:?}");
        }
    });
}

fn extract_text(webview: &WebView, event_tx: mpsc::SyncSender<RuntimeEvent>) {
    webview.evaluate_javascript(EXTRACTION_SCRIPT, move |result| match result {
        Ok(value) => {
            let nodes = parse_js_nodes(&value);
            if !nodes.is_empty() {
                let _ = event_tx.try_send(RuntimeEvent::TextNodes(nodes));
            }
        }
        Err(e) => {
            if !matches!(e, JavaScriptEvaluationError::WebViewNotReady) {
                warn!("text extraction failed: {e:?}");
            }
        }
    });
}

fn paint(webview: &WebView, ctx: &dyn RenderingContext) -> Option<BrowserFrame> {
    use glam::UVec2;

    ctx.make_current().ok()?;
    webview.paint();

    let size = ctx.size();
    let rect = DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(size.width as i32, size.height as i32),
    );

    let image = ctx.read_to_image(rect)?;
    ctx.present();

    Some(BrowserFrame {
        size: UVec2::new(image.width(), image.height()),
        pixels: image.into_raw(),
    })
}

fn browser_preferences(mut p: Preferences) -> Preferences {
    p.network_http_proxy_uri.clear();
    p.network_https_proxy_uri.clear();
    p.network_http_no_proxy.clear();
    p
}
