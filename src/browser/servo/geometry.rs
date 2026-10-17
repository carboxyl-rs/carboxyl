//! Mapping terminal cells onto the Servo viewport.

use dpi::PhysicalSize;
use glam::UVec2;
use servo::{DevicePoint, WebViewPoint};

use crate::output::{NAV_BAR_ROWS, Window};

/// The centre of the pixel block a terminal cell shows.
///
/// The viewport is an exact multiple of the cell size, so this is exact rather
/// than approximate: a click always lands on the pixels its cell displays.
/// Returns `None` for the navigation bar row, which is not part of the page.
pub fn viewport_point(window: &Window, col: u16, row: u16) -> Option<WebViewPoint> {
    let row = row.checked_sub(NAV_BAR_ROWS)?;
    let viewport = window.viewport();

    let x = (f32::from(col) + 0.5) * window.cell.x as f32;
    let y = (f32::from(row) + 0.5) * window.cell.y as f32;
    if x >= viewport.x as f32 || y >= viewport.y as f32 {
        return None;
    }

    Some(WebViewPoint::Device(DevicePoint::new(x, y)))
}

pub fn physical_size(size: UVec2) -> PhysicalSize<u32> {
    PhysicalSize::new(size.x, size.y)
}
