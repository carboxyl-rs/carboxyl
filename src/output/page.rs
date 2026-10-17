//! A page snapshot containing pixels and text from the same Servo turn.

use std::sync::Arc;

use glam::UVec2;

/// A rectangle in viewport device pixels, relative to the browser viewport's
/// top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PxRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl PxRect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    /// Finite, non-degenerate, and at least partly inside a `size` viewport.
    pub fn is_visible_in(&self, size: UVec2) -> bool {
        [self.x0, self.y0, self.x1, self.y1]
            .iter()
            .all(|value| value.is_finite())
            && self.width() > 0.0
            && self.height() > 0.0
            && self.x1 > 0.0
            && self.y1 > 0.0
            && self.x0 < size.x as f32
            && self.y0 < size.y as f32
    }
}

/// Text owned directly by one element; boxed descendants get separate entries.
/// Deeper boxes reserve their cells first so enclosing text flows around them.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    pub rect: PxRect,
    pub text: String,
    pub depth: u16,
}

/// Frame pixels and text boxes, shared until the accessibility tree changes.
#[derive(Clone)]
pub struct PageFrame {
    /// Viewport size in device pixels. Always an exact multiple of the cell size.
    pub size: UVec2,
    /// RGBA8888, `size.x * size.y * 4` bytes.
    pub pixels: Vec<u8>,
    /// Text boxes in document order.
    pub text: Arc<[TextBox]>,
}

impl PageFrame {
    /// Whether pixels and text contents match `other`.
    pub fn matches(&self, other: &Self) -> bool {
        self.size == other.size
            && same(&self.text, &other.text)
            && self.pixels == other.pixels
    }
}

/// Compare shared allocations first, then separately allocated contents.
fn same<T: PartialEq>(a: &Arc<[T]>, b: &Arc<[T]>) -> bool {
    Arc::ptr_eq(a, b) || a == b
}
