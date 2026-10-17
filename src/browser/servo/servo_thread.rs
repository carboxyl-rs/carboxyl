use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dpi::PhysicalSize;
use glam::UVec2;
use log::error;
use servo::{
    DeviceIntPoint, DeviceIntRect, DeviceIntSize, Preferences, RenderingContext, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
};
use url::Url;

use crate::output::PageFrame;

use super::a11y::Accessibility;
use super::delegates::{TerminalServoDelegate, TerminalWebViewDelegate};
use super::events::{RuntimeEvent, ServoCommand};
use super::waker::ServoWaker;

/// Yield between Servo turns while keeping timers and animations responsive.
const SPIN_SLEEP: Duration = Duration::from_millis(1);

/// Live documents per history direction; older entries reload on navigation.
const SESSION_HISTORY_MAX_LENGTH: i64 = 3;

/// How the Servo thread should start up.
pub struct Setup {
    pub url: Url,
    pub viewport: PhysicalSize<u32>,
    pub zoom: f32,
    pub native_text: bool,
}

/// Commands accumulated while draining the queue in one pass, so a burst is
/// handled in a single Servo turn rather than one turn each.
#[derive(Default)]
struct Pending {
    shutdown: bool,
    paint: bool,
    resize: Option<PhysicalSize<u32>>,
}

impl Pending {
    fn apply(&mut self, command: ServoCommand, webview: &WebView) {
        match command {
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
            ServoCommand::Input(event) => {
                webview.notify_input_event(event);
            }
            ServoCommand::Paint => self.paint = true,
            ServoCommand::Wake => {}
        }
    }
}

pub fn servo_thread(
    event_tx: mpsc::SyncSender<RuntimeEvent>,
    servo_tx: mpsc::SyncSender<ServoCommand>,
    servo_rx: mpsc::Receiver<ServoCommand>,
    setup: Setup,
) {
    let servo = ServoBuilder::default()
        .preferences(preferences(setup.native_text))
        .event_loop_waker(Box::new(ServoWaker::new(servo_tx)))
        .build();

    servo.set_delegate(Rc::new(TerminalServoDelegate));

    let rendering_context: Rc<dyn RenderingContext> =
        match SoftwareRenderingContext::new(setup.viewport) {
            Ok(context) => Rc::new(context),
            Err(error) => {
                error!("failed to create rendering context: {error:?}");
                let _ = event_tx.try_send(RuntimeEvent::Exit);
                return;
            }
        };

    let a11y = Rc::new(RefCell::new(None));
    let delegate: Rc<dyn WebViewDelegate> = Rc::new(TerminalWebViewDelegate {
        event_tx: event_tx.clone(),
        a11y: a11y.clone(),
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate)
        .url(setup.url)
        .build();

    webview.show();
    webview.set_focused(true);
    webview.set_page_zoom(setup.zoom);

    if setup.native_text {
        // The graft must exist before any update from Servo is applied, so the
        // tree is built here from the id activation hands back.
        match webview.set_accessibility_active(true) {
            Some(tree_id) => *a11y.borrow_mut() = Some(Accessibility::new(tree_id)),
            None => error!("servo declined to activate accessibility; text stays rasterised"),
        }
    }

    while let Ok(command) = servo_rx.recv() {
        let mut pending = Pending::default();
        pending.apply(command, &webview);
        while let Ok(command) = servo_rx.try_recv() {
            pending.apply(command, &webview);
        }

        if pending.shutdown {
            break;
        }

        if let Some(size) = pending.resize {
            webview.resize(size);
        }

        servo.spin_event_loop();

        if pending.paint
            && let Some(frame) = paint(&webview, rendering_context.as_ref(), &a11y)
        {
            let _ = event_tx.try_send(RuntimeEvent::Frame(frame));
        }

        thread::sleep(SPIN_SLEEP);
    }
}

/// Read pixels and text from the same Servo turn to keep scroll positions aligned.
fn paint(
    webview: &WebView,
    context: &dyn RenderingContext,
    a11y: &RefCell<Option<Accessibility>>,
) -> Option<PageFrame> {
    context.make_current().ok()?;
    webview.paint();

    let size = context.size();
    let image = context.read_to_image(DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(size.width as i32, size.height as i32),
    ))?;
    context.present();

    let (text, images) = a11y
        .borrow_mut()
        .as_mut()
        .map(Accessibility::snapshot)
        .unwrap_or_else(|| (Vec::new().into(), Vec::new().into()));

    Some(PageFrame {
        size: UVec2::new(image.width(), image.height()),
        pixels: image.into_raw(),
        text,
        images,
    })
}

fn preferences(native_text: bool) -> Preferences {
    Preferences {
        session_history_max_length: SESSION_HISTORY_MAX_LENGTH,
        // Off by default upstream, and all that native text needs.
        accessibility_enabled: native_text,
        ..Preferences::default()
    }
}
