//! Shared terminal color utilities.
//!
//! Both the pixel renderer ([`BrowserWidget`]) and the text overlay
//! ([`TextOverlay`]) need BT.601 luma coefficients and the xterm 6×6×6
//! color-cube mapping. Keeping them here prevents the two from drifting.

use ratatui::style::Color;

// ---------------------------------------------------------------------------
// BT.601 luma coefficients (integer fixed-point, ×256)
// ---------------------------------------------------------------------------

/// BT.601 luma coefficient for red, scaled by 256.
/// Exact: 0.299 × 256 = 76.544 — rounded to 77.
pub const LUMA_R: u32 = 77;

/// BT.601 luma coefficient for green, scaled by 256.
/// Exact: 0.587 × 256 = 150.272 — rounded to 150.
pub const LUMA_G: u32 = 150;

/// BT.601 luma coefficient for blue, scaled by 256.
/// Exact: 0.114 × 256 = 29.184 — rounded to 29.
pub const LUMA_B: u32 = 29;

/// Right-shift amount that undoes the ×256 fixed-point scale, recovering
/// a luma value in the 0..=255 range.
pub const LUMA_SHIFT: u32 = 8;

/// Maximum luma value on the pre-shift scale: white = 255 × (77 + 150 + 29)
/// = 255 × 256 = 65 280.
pub const MAX_LUMA: u32 = 255 * (LUMA_R + LUMA_G + LUMA_B);

// ---------------------------------------------------------------------------
// xterm 6×6×6 color cube
// ---------------------------------------------------------------------------

/// First palette index of the 6×6×6 color cube.
/// Indices 0–15 are the 16 ANSI named colors; the cube starts at 16.
pub const CUBE_BASE: u8 = 16;

/// Number of discrete intensity steps (0–5) per channel in the cube.
pub const CUBE_STEPS: u8 = 6;

/// Stride of the red channel within the cube (CUBE_STEPS² = 36).
pub const CUBE_RED_STRIDE: u8 = CUBE_STEPS * CUBE_STEPS;

/// Maps an 8-bit channel value into a 0–5 cube index.
/// 256 / 6 ≈ 42.67; 43 gives the correct bucket boundaries.
pub const CUBE_CHANNEL_DIVISOR: u8 = 43;

// Compile-time check that the stride constant matches the formula.
const _CUBE_STRIDE_CHECK: () =
    assert!(CUBE_RED_STRIDE as u16 == CUBE_STEPS as u16 * CUBE_STEPS as u16);

// ---------------------------------------------------------------------------
// Color conversion
// ---------------------------------------------------------------------------

/// Convert an 8-bit RGB triple to a ratatui [`Color`].
///
/// Emits [`Color::Rgb`] (24-bit) when the terminal supports true color, or
/// the nearest entry in the xterm 6×6×6 256-color cube otherwise.
pub fn to_terminal_color((r, g, b): (u8, u8, u8), true_color: bool) -> Color {
    if true_color {
        Color::Rgb(r, g, b)
    } else {
        let q = |v: u8| (v / CUBE_CHANNEL_DIVISOR).min(CUBE_STEPS - 1);
        Color::Indexed(CUBE_BASE + q(r) * CUBE_RED_STRIDE + q(g) * CUBE_STEPS + q(b))
    }
}
