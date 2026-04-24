use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use dpi::PhysicalSize;
use servo::{
    EventLoopWaker, LoadStatus, RenderingContext, ServoBuilder, SoftwareRenderingContext, WebView,
    WebViewBuilder, WebViewDelegate,
};
use url::Url;

#[derive(Clone)]
struct TestWaker(Arc<AtomicBool>);

impl EventLoopWaker for TestWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Delegate {
    load_complete: Cell<bool>,
    new_frame_ready: Cell<bool>,
}

impl WebViewDelegate for Delegate {
    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        if status == LoadStatus::Complete {
            self.load_complete.set(true);
        }
    }

    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.new_frame_ready.set(true);
    }
}

fn spin_until(servo: &servo::Servo, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);

    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for Servo state"
        );
        servo.spin_event_loop();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn software_rendering_pump_loop_eventually_contains_page_pixels() {
    let rendering_context: Rc<dyn RenderingContext> = Rc::new(
        SoftwareRenderingContext::new(PhysicalSize::new(128, 128))
            .expect("Could not create SoftwareRenderingContext"),
    );
    rendering_context
        .make_current()
        .expect("Could not make SoftwareRenderingContext current");

    let waker = Arc::new(AtomicBool::new(false));
    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(TestWaker(waker)))
        .build();
    let delegate = Rc::new(Delegate::default());
    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .delegate(delegate.clone())
        .url(
            Url::parse(
                "data:text/html;base64,PCFkb2N0eXBlIGh0bWw+PHN0eWxlPmh0bWwsYm9keXttYXJnaW46MDt3aWR0aDoxMDAlO2hlaWdodDoxMDAlO2JhY2tncm91bmQ6I2YwMDt9PC9zdHlsZT4=",
            )
            .expect("valid data URL"),
        )
        .build();

    webview.show();

    spin_until(&servo, || {
        delegate.load_complete.get() && delegate.new_frame_ready.get()
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut rendered_red = false;

    while Instant::now() < deadline {
        servo.spin_event_loop();

        if delegate.new_frame_ready.replace(false) {
            rendering_context
                .make_current()
                .expect("Could not make SoftwareRenderingContext current");
            webview.paint();

            if let Some(image) =
                rendering_context.read_to_image(servo::DeviceIntRect::from_origin_and_size(
                    servo::DeviceIntPoint::new(0, 0),
                    servo::DeviceIntSize::new(128, 128),
                ))
            {
                let pixel = image.get_pixel(64, 64).0;
                if pixel[0] > 200 && pixel[1] < 80 && pixel[2] < 80 {
                    rendered_red = true;
                    break;
                }
            }

            rendering_context.present();
        }

        thread::sleep(Duration::from_millis(1));
    }

    assert!(rendered_red, "expected the pump loop to render a red page");
}
