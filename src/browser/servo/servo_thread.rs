use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dpi::PhysicalSize;
use log::error;
use servo::{
    DeviceIntPoint, DeviceIntRect, DeviceIntSize, Preferences, RenderingContext, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
};
use url::Url;

use crate::output::BrowserFrame;

use super::delegates::{TerminalServoDelegate, TerminalWebViewDelegate};
use super::events::{RuntimeEvent, ServoCommand};
use super::waker::ServoWaker;

// ---------------------------------------------------------------------------
// Timing constants
// ---------------------------------------------------------------------------

/// Brief sleep after each `spin_event_loop` call. Prevents the servo thread
/// from monopolising a core between paint cycles while still allowing
/// Servo's own timers and animation frames to fire promptly.
const SERVO_SPIN_SLEEP: Duration = Duration::from_millis(1);

// ---------------------------------------------------------------------------
// PendingOps - batch-accumulates commands drained from the servo channel
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PendingOps {
    shutdown: bool,
    paint: bool,
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
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate)
        .url(url)
        .build();

    webview.show();
    webview.focus();

    while let Ok(cmd) = servo_rx.recv() {
        let mut ops = PendingOps::default();
        ops.apply(cmd, &webview);
        while let Ok(cmd) = servo_rx.try_recv() {
            ops.apply(cmd, &webview);
        }

        if ops.shutdown {
            break;
        }

        if let Some(size) = ops.resize {
            webview.resize(size);
        }

        servo.spin_event_loop();

        if ops.paint
            && let Some(frame) = paint(&webview, rendering_context.as_ref())
        {
            let _ = event_tx.try_send(RuntimeEvent::Frame(frame));
        }

        thread::sleep(SERVO_SPIN_SLEEP);
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn paint(webview: &WebView, ctx: &dyn RenderingContext) -> Option<BrowserFrame> {
    use glam::{UVec2, Vec2};

    ctx.make_current().ok()?;
    webview.paint();

    let size = ctx.size();
    let rect = DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(size.width as i32, size.height as i32),
    );

    let image = ctx.read_to_image(rect)?;
    ctx.present();

    // Read the offset the renderer just composited with, so the overlay aligns
    // with these exact pixels regardless of async-scroll timing.
    let scroll_offset = webview
        .root_scroll_offset()
        .map(|(x, y)| Vec2::new(x, y))
        .unwrap_or(Vec2::ZERO);

    Some(BrowserFrame {
        size: UVec2::new(image.width(), image.height()),
        pixels: image.into_raw(),
        scroll_offset,
    })
}

fn browser_preferences(mut p: Preferences) -> Preferences {
    p.network_http_proxy_uri.clear();
    p.network_https_proxy_uri.clear();
    p.network_http_no_proxy.clear();
    // Capture the display list so the overlay knows where the text is and
    // its real color, and stop Servo from rasterizing the glyphs itself so
    // the overlay is the only text on screen - no doubling, no color loss.
    p.layout_display_list_capture_enabled = true;
    p.layout_text_painting_enabled = false;
    p
}
