//! Headless end-to-end check: real Servo layout, real pixels, real composition.
//!
//! ```console
//! cargo run --release --example render_smoke -- target/render-smoke
//! ```
//!
//! Loads `tests/fixtures/rendering.html`, which is built to exercise the cases
//! that are easy to get wrong - wrapped paragraphs, inline links between runs
//! of text, a fixed header, content hidden by `opacity`, content covered by an
//! opaque element, preformatted text and an image - then checks the composed
//! terminal output, scrolls, resizes and navigates away. Needs no network and
//! no window server. A text snapshot of each screen is written to the output
//! directory.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use carboxyl::browser::Accessibility;
use carboxyl::output::{Canvas, ComposeOptions, PageFrame};
use dpi::PhysicalSize;
use glam::UVec2;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use servo::accesskit::TreeUpdate;
use servo::{
    DeviceIntPoint, DeviceIntRect, DeviceIntSize, DevicePoint, DeviceVector2D, EventLoopWaker,
    LoadStatus, Preferences, RenderingContext, Scroll, Servo, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate, WebViewPoint,
    WebViewVector,
};
use url::Url;

/// Cell size the check renders at: the common 8x16, which is also what the real
/// binary uses when a terminal reports nothing better.
const CELL: UVec2 = UVec2::new(8, 16);

/// Longest wait for any one Servo operation.
const DEADLINE: Duration = Duration::from_secs(60);
/// How long the page must stop changing before a snapshot is taken.
const SETTLED: Duration = Duration::from_millis(400);

fn main() -> Result<()> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/render-smoke".to_owned()),
    );
    std::fs::create_dir_all(&out)?;

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rendering.html");
    let url =
        Url::from_file_path(&fixture).map_err(|_| anyhow!("fixture path is not a file URL"))?;

    let mut page = Page::new(100, 40, url.clone())?;
    page.await_load(&url)?;

    let loaded = page.snapshot(&out, "loaded")?;
    loaded.expect(&[
        // Wrapped paragraphs keep every word.
        "Servo, in your terminal.",
        // Unicode, including graphemes wider than one cell.
        "Spacing: one two three.",
        // Preformatted text keeps its own line breaks.
        "let browser = Servo::new();",
        "browser.paint();",
        // A nested scroller's rows are laid out like any other box.
        "Nested first row",
    ])?;
    loaded.expect_wrapped("Wrapped paragraphs should keep every word, including the final phrase. Native glyphs stay readable while Servo handles layout, backgrounds, images, and scrolling.")?;
    loaded.reject(&[
        // Both sentinels are in the accessibility tree with real geometry.
        // Neither is visible, so neither may be drawn.
        "INVISIBLE SENTINEL",
        "OCCLUDED SENTINEL",
    ])?;
    // Inline layout: a link between two runs of text in one box, in order.
    loaded.order(&["First sibling", "linked words", "last sibling."])?;
    // The fixed header is at the top of the screen.
    loaded.near_top("CARBOXYL")?;

    // Scrolling: the document moves, the fixed header does not.
    page.scroll(5000.0)?;
    let scrolled = page.snapshot(&out, "scrolled")?;
    scrolled.near_top("CARBOXYL")?;
    scrolled.expect(&["Bottom of the document. The fixed header stays in place."])?;
    scrolled.reject(&["Servo, in your terminal."])?;

    // A resize reflows the page and the text follows it.
    page.resize(60, 30)?;
    let resized = page.snapshot(&out, "resized")?;
    if resized.buffer.area.width != 60 {
        bail!("the resize never reached the renderer");
    }
    resized.near_top("CARBOXYL")?;

    // Navigating away clears the previous page's text.
    page.load(Url::parse("about:blank")?)?;
    page.snapshot(&out, "blank")?
        .reject(&["CARBOXYL", "linked words"])?;

    println!("render_smoke: ok, snapshots in {}", out.display());
    Ok(())
}

struct Page {
    servo: Servo,
    webview: WebView,
    context: Rc<dyn RenderingContext>,
    a11y: Rc<RefCell<Option<Accessibility>>>,
    progress: Rc<Progress>,
    canvas: Canvas,
    cells: UVec2,
}

/// Counters the delegate bumps, so the harness can tell when a page has
/// stopped changing.
#[derive(Default)]
struct Progress {
    loaded: Cell<bool>,
    changes: Cell<u64>,
}

struct Delegate {
    progress: Rc<Progress>,
    a11y: Rc<RefCell<Option<Accessibility>>>,
}

impl Delegate {
    fn changed(&self) {
        self.progress.changes.set(self.progress.changes.get() + 1);
    }
}

impl WebViewDelegate for Delegate {
    fn notify_load_status_changed(&self, _: WebView, status: LoadStatus) {
        self.progress.loaded.set(status == LoadStatus::Complete);
        self.changed();
    }

    fn notify_new_frame_ready(&self, _: WebView) {
        self.changed();
    }

    fn notify_accessibility_tree_update(&self, _: WebView, update: TreeUpdate) {
        if let Some(a11y) = self.a11y.borrow_mut().as_mut() {
            a11y.update(update);
        }
        self.changed();
    }
}

#[derive(Clone)]
struct Waker(thread::Thread);

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        self.0.unpark();
    }
}

impl Page {
    fn new(cols: u32, rows: u32, url: Url) -> Result<Self> {
        let preferences = Preferences {
            accessibility_enabled: true,
            ..Preferences::default()
        };

        let servo = ServoBuilder::default()
            .preferences(preferences)
            .event_loop_waker(Box::new(Waker(thread::current())))
            .build();

        let cells = UVec2::new(cols, rows);
        let viewport = cells * CELL;
        let context: Rc<dyn RenderingContext> = Rc::new(
            SoftwareRenderingContext::new(PhysicalSize::new(viewport.x, viewport.y))
                .map_err(|error| anyhow!("rendering context: {error:?}"))?,
        );

        let progress = Rc::new(Progress::default());
        let a11y = Rc::new(RefCell::new(None));
        let webview = WebViewBuilder::new(&servo, context.clone())
            .delegate(Rc::new(Delegate {
                progress: progress.clone(),
                a11y: a11y.clone(),
            }))
            .url(url)
            .build();

        webview.show();
        webview.set_focused(true);
        let tree_id = webview
            .set_accessibility_active(true)
            .context("servo declined to activate accessibility")?;
        *a11y.borrow_mut() = Some(Accessibility::new(tree_id));

        Ok(Self {
            servo,
            webview,
            context,
            a11y,
            progress,
            canvas: Canvas::default(),
            cells,
        })
    }

    fn load(&mut self, url: Url) -> Result<()> {
        self.progress.loaded.set(false);
        self.webview.load(url.clone());
        self.await_load(&url)
    }

    fn await_load(&mut self, url: &Url) -> Result<()> {
        let deadline = Instant::now() + DEADLINE;
        // Ignore a late load completion from the previous about:blank page.
        while !(self.progress.loaded.get() && self.webview.url().as_ref() == Some(url)) {
            if Instant::now() > deadline {
                bail!(
                    "timed out loading {url} (at {:?}, {:?}, loaded flag {})",
                    self.webview.url(),
                    self.webview.load_status(),
                    self.progress.loaded.get()
                );
            }
            self.servo.spin_event_loop();
            thread::park_timeout(Duration::from_millis(2));
        }
        self.settle();
        Ok(())
    }

    /// Scroll the root viewport down by `delta` device pixels, like
    /// `window.scrollBy`. The point is to the right of the fixture's content
    /// column, so the root scrolls rather than the nested scroller.
    fn scroll(&mut self, delta: f32) -> Result<()> {
        let x = (self.cells.x * CELL.x) as f32 - 8.0;
        self.webview.notify_scroll_event(
            Scroll::Delta(WebViewVector::Device(DeviceVector2D::new(0.0, delta))),
            WebViewPoint::Device(DevicePoint::new(x, 400.0)),
        );
        self.settle();
        Ok(())
    }

    fn resize(&mut self, cols: u32, rows: u32) -> Result<()> {
        self.cells = UVec2::new(cols, rows);
        let viewport = self.cells * CELL;
        self.webview
            .resize(PhysicalSize::new(viewport.x, viewport.y));
        self.settle();
        Ok(())
    }

    /// Spin Servo until nothing has changed for [`SETTLED`], so a snapshot is
    /// of a finished page rather than a half-laid-out one.
    fn settle(&mut self) {
        let deadline = Instant::now() + DEADLINE;
        let mut quiet = Instant::now();
        let mut seen = self.progress.changes.get();
        while quiet.elapsed() < SETTLED && Instant::now() < deadline {
            self.servo.spin_event_loop();
            self.webview.paint();
            if self.progress.changes.get() != seen {
                seen = self.progress.changes.get();
                quiet = Instant::now();
            }
            thread::park_timeout(Duration::from_millis(5));
        }
    }

    /// Composite a frame and compose it, exactly as the real session does.
    fn snapshot(&mut self, out: &Path, name: &str) -> Result<Screen> {
        self.context
            .make_current()
            .map_err(|error| anyhow!("make_current: {error:?}"))?;
        self.webview.paint();

        let size = self.context.size();
        let image = self
            .context
            .read_to_image(DeviceIntRect::from_origin_and_size(
                DeviceIntPoint::new(0, 0),
                DeviceIntSize::new(size.width as i32, size.height as i32),
            ))
            .context("reading back the frame")?;
        self.context.present();

        let (text, images) = self
            .a11y
            .borrow_mut()
            .as_mut()
            .map(Accessibility::snapshot)
            .unwrap_or_else(|| (Vec::new().into(), Vec::new().into()));
        let frame = PageFrame {
            size: UVec2::new(image.width(), image.height()),
            pixels: image.into_raw(),
            text,
            images,
        };
        std::fs::write(
            out.join(format!("{name}-boxes.txt")),
            format!("{:#?}", frame.text),
        )?;

        let area = Rect::new(0, 0, self.cells.x as u16, self.cells.y as u16);
        let mut buffer = Buffer::empty(area);
        self.canvas.compose(
            area,
            &mut buffer,
            &frame,
            CELL,
            ComposeOptions {
                true_color: true,
                native_text: true,
            },
        );

        let screen = Screen {
            buffer,
            name: name.to_owned(),
        };
        std::fs::write(out.join(format!("{name}.txt")), screen.text())?;
        Ok(screen)
    }
}

struct Screen {
    buffer: Buffer,
    name: String,
}

impl Screen {
    /// Rows with runs of whitespace collapsed, which is how text reads once it
    /// has been laid out on a fixed grid.
    fn rows(&self) -> Vec<String> {
        (0..self.buffer.area.height)
            .map(|row| {
                let line: String = (0..self.buffer.area.width)
                    .map(|col| self.buffer[(col, row)].symbol())
                    .collect();
                line.split_whitespace().collect::<Vec<_>>().join(" ")
            })
            .collect()
    }

    fn text(&self) -> String {
        let mut text = self.rows().join("\n");
        text.push('\n');
        text
    }

    fn expect(&self, needles: &[&str]) -> Result<()> {
        let rows = self.rows();
        for needle in needles {
            let flat = needle.split_whitespace().collect::<Vec<_>>().join(" ");
            if !rows.iter().any(|row| row.contains(&flat)) {
                bail!("{}: expected {needle:?}\n{}", self.name, self.text());
            }
        }
        Ok(())
    }

    fn reject(&self, needles: &[&str]) -> Result<()> {
        let rows = self.rows();
        for needle in needles {
            if let Some(row) = rows.iter().find(|row| row.contains(needle)) {
                bail!(
                    "{}: {needle:?} is not visible, but was drawn on {row:?}",
                    self.name
                );
            }
        }
        Ok(())
    }

    /// Wrapped prose may cross terminal rows; all words must still survive.
    fn expect_wrapped(&self, needle: &str) -> Result<()> {
        let text = self.text().split_whitespace().collect::<Vec<_>>().join(" ");
        let flat = needle.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.contains(&flat) {
            bail!("{}: expected {needle:?}\n{}", self.name, self.text());
        }
        Ok(())
    }

    /// All of `needles` on one row, in this order: the check that inline
    /// siblings around a nested box come out in document order.
    fn order(&self, needles: &[&str]) -> Result<()> {
        'rows: for row in self.rows() {
            let mut at = 0;
            for needle in needles {
                let Some(found) = row[at..].find(needle) else {
                    continue 'rows;
                };
                at += found + needle.len();
            }
            return Ok(());
        }
        bail!(
            "{}: no row held {needles:?} in order\n{}",
            self.name,
            self.text()
        );
    }

    /// Somewhere in the first few rows, which is where a fixed header lands
    /// whatever its padding.
    fn near_top(&self, needle: &str) -> Result<()> {
        let rows = self.rows();
        let top = rows.get(..3).unwrap_or(&rows);
        if top.iter().any(|row| row.contains(needle)) {
            return Ok(());
        }
        bail!(
            "{}: expected {needle:?} near the top, got {top:?}",
            self.name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped_screen() -> Screen {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 2));
        buffer.set_string(0, 0, "Native glyphs", ratatui::style::Style::default());
        buffer.set_string(0, 1, "stay readable", ratatui::style::Style::default());
        Screen {
            buffer,
            name: "wrapped".to_owned(),
        }
    }

    #[test]
    fn wrapped_expectation_accepts_a_row_break() {
        assert!(
            wrapped_screen()
                .expect_wrapped("Native glyphs stay readable")
                .is_ok()
        );
    }

    #[test]
    fn wrapped_expectation_rejects_missing_words() {
        assert!(
            wrapped_screen()
                .expect_wrapped("Native glyphs always stay readable")
                .is_err()
        );
    }

    #[test]
    fn wrapped_expectation_accepts_blank_rows_between_lines() {
        let mut screen = wrapped_screen();
        screen.buffer.resize(Rect::new(0, 0, 20, 3));
        screen.buffer.set_string(
            0,
            1,
            "                    ",
            ratatui::style::Style::default(),
        );
        screen
            .buffer
            .set_string(0, 2, "stay readable", ratatui::style::Style::default());
        assert!(screen.expect_wrapped("Native glyphs stay readable").is_ok());
    }
}
