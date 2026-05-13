use dpi::PhysicalSize;
use glam::{UVec2, Vec2};
use servo::{DevicePoint, WebViewPoint};

use crate::output::Window;

#[derive(Clone, Copy, Debug, Default)]
pub struct BrowserPoint(Vec2);

impl BrowserPoint {
    pub fn from_cell(window: &Window, col: u16, row: u16) -> Self {
        Self(Vec2::new(
            ((col as f32 + 0.5) * window.cell_pixels.x).floor(),
            ((row as f32 - 0.5) * window.cell_pixels.y).floor(),
        ))
    }

    pub fn to_webview_point(self) -> WebViewPoint {
        WebViewPoint::Device(DevicePoint::new(self.0.x, self.0.y))
    }
}

pub fn physical_size(size: UVec2) -> PhysicalSize<u32> {
    PhysicalSize::new(size.x, size.y)
}
