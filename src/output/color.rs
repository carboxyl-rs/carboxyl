//! Shared terminal color utilities.

use ratatui::style::Color;

/// One opaque pixel. Servo's alpha channel is dropped: the frame is already
/// composited against the page's canvas.
pub type Rgb = (u8, u8, u8);

/// BT.601 luma coefficient for red, scaled by 256 (0.299 x 256 = 76.5).
const LUMA_R: u32 = 77;
/// BT.601 luma coefficient for green, scaled by 256 (0.587 x 256 = 150.3).
const LUMA_G: u32 = 150;
/// BT.601 luma coefficient for blue, scaled by 256 (0.114 x 256 = 29.2).
const LUMA_B: u32 = 29;

// The coefficients must sum to exactly 256 so that `luma` of white is 255.
const _LUMA_SUM_CHECK: () = assert!(LUMA_R + LUMA_G + LUMA_B == 256);

/// Perceptual brightness in the 0..=255 range.
pub fn luma((r, g, b): Rgb) -> u32 {
    (u32::from(r) * LUMA_R + u32::from(g) * LUMA_G + u32::from(b) * LUMA_B) >> 8
}

/// First palette index of the 6x6x6 color cube; 0-15 are the named colors.
const CUBE_BASE: u8 = 16;
/// Discrete intensity steps (0-5) per channel in the cube.
const CUBE_STEPS: u8 = 6;
/// Stride of the red channel within the cube.
const CUBE_RED_STRIDE: u8 = CUBE_STEPS * CUBE_STEPS;
/// Maps an 8-bit channel value into a 0-5 cube index (256 / 6 = 42.7).
const CUBE_CHANNEL_DIVISOR: u8 = 43;

/// Convert an 8-bit RGB triple to a ratatui [`Color`].
///
/// Emits 24-bit color when the terminal supports it, otherwise the nearest
/// entry in the xterm 6x6x6 cube.
pub fn to_terminal_color((r, g, b): Rgb, true_color: bool) -> Color {
    if true_color {
        Color::Rgb(r, g, b)
    } else {
        let quantize = |value: u8| (value / CUBE_CHANNEL_DIVISOR).min(CUBE_STEPS - 1);
        Color::Indexed(
            CUBE_BASE + quantize(r) * CUBE_RED_STRIDE + quantize(g) * CUBE_STEPS + quantize(b),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_spans_the_full_range() {
        assert_eq!(luma((0, 0, 0)), 0);
        assert_eq!(luma((255, 255, 255)), 255);
        assert!(luma((0, 255, 0)) > luma((255, 0, 0)));
    }

    #[test]
    fn palette_fallback_stays_inside_the_color_cube() {
        assert_eq!(to_terminal_color((1, 2, 3), true), Color::Rgb(1, 2, 3));
        for channel in 0..=255u8 {
            let rgb = (channel, channel, channel);
            let Color::Indexed(index) = to_terminal_color(rgb, false) else {
                panic!("expected an indexed color");
            };
            assert!((CUBE_BASE..CUBE_BASE + 216).contains(&index));
        }
    }
}
