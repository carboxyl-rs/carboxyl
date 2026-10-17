//! Draw detailed regions with terminal graphics and cache encodings by content.
//! `ratatui-image` handles protocol detection; quadrant blocks provide a fallback.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use glam::UVec2;
use image::{DynamicImage, RgbaImage};
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Widget;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use crate::cli::GraphicsMode;

use super::page::PageFrame;

/// Limit retained encodings to avoid exhausting the terminal's image store.
const MAX_CACHED: usize = 48;

/// Regions larger than this are left as quadrant blocks: encoding them costs
/// more than the detail is worth.
const MAX_TILE_PIXELS: u32 = 4 << 20;

/// A detected terminal graphics protocol, with its encoded-region cache.
pub struct Graphics {
    picker: Picker,
    cache: HashMap<u64, Protocol>,
    live: Vec<u64>,
}

impl Graphics {
    /// Query the terminal for a graphics protocol and its cell size.
    ///
    /// Must run after the alternate screen is entered and before anything else
    /// reads terminal input, because the query is a round trip over stdio.
    pub fn detect(mode: GraphicsMode) -> Option<Self> {
        if mode == GraphicsMode::Quadrant {
            return None;
        }

        let picker = Picker::from_query_stdio()
            .inspect_err(|error| log::info!("no terminal graphics protocol: {error}"))
            .ok()?;

        let detected = picker.protocol_type();
        if detected == ProtocolType::Halfblocks {
            // Half blocks are what `ratatui-image` falls back to when the
            // terminal answered nothing useful. Carboxyl's own quadrant blocks
            // have twice the horizontal resolution, so prefer those.
            log::info!("terminal reported no graphics protocol, using quadrant blocks");
            return None;
        }

        let requested = match mode {
            GraphicsMode::Kitty => Some(ProtocolType::Kitty),
            GraphicsMode::Sixel => Some(ProtocolType::Sixel),
            GraphicsMode::Auto | GraphicsMode::Quadrant => None,
        };
        if let Some(requested) = requested
            && requested != detected
        {
            log::warn!(
                "terminal does not support {requested:?} graphics (it reports {detected:?}), \
                 using quadrant blocks"
            );
            return None;
        }

        log::info!(
            "terminal graphics: {detected:?}, cell {}x{}",
            picker.font_size().width,
            picker.font_size().height
        );
        Some(Self {
            picker,
            cache: HashMap::new(),
            live: Vec::new(),
        })
    }

    /// The terminal's cell size in pixels, which the viewport is sized to so
    /// that a region is a straight crop of Servo's buffer.
    pub fn cell_size(&self) -> UVec2 {
        let size = self.picker.font_size();
        UVec2::new(u32::from(size.width), u32::from(size.height))
    }

    /// Draw `tiles` of `frame` over `area`.
    ///
    /// Tiles come from the composer and never overlap a native glyph, so each
    /// one can be handed to the protocol whole.
    pub fn render(
        &mut self,
        frame: &PageFrame,
        cell: UVec2,
        tiles: &[Rect],
        area: Rect,
        buf: &mut Buffer,
    ) {
        self.live.clear();

        for tile in tiles {
            let Some(key) = fingerprint(frame, cell, *tile) else {
                continue;
            };

            if !self.cache.contains_key(&key) {
                let Some(protocol) = encode(&self.picker, frame, cell, *tile) else {
                    continue;
                };
                self.cache.insert(key, protocol);
            }

            self.live.push(key);
            if let Some(protocol) = self.cache.get(&key) {
                let placed = Rect::new(area.x + tile.x, area.y + tile.y, tile.width, tile.height);
                Image::new(protocol).render(placed, buf);
            }
        }

        if self.cache.len() > MAX_CACHED {
            let live = &self.live;
            self.cache.retain(|key, _| live.contains(key));
        }
    }
}

/// Identify a region by the pixels in it, so the same image is uploaded once
/// however often it moves or recurs on the page.
fn fingerprint(frame: &PageFrame, cell: UVec2, tile: Rect) -> Option<u64> {
    let (origin, size) = tile_pixels(cell, tile);
    if size.x == 0
        || size.y == 0
        || origin.x + size.x > frame.size.x
        || origin.y + size.y > frame.size.y
    {
        return None;
    }

    let mut hasher = DefaultHasher::new();
    size.x.hash(&mut hasher);
    size.y.hash(&mut hasher);
    let stride = frame.size.x as usize * 4;
    for row in 0..size.y {
        let start = (origin.y + row) as usize * stride + origin.x as usize * 4;
        hasher.write(frame.pixels.get(start..start + size.x as usize * 4)?);
    }
    Some(hasher.finish())
}

/// Encode one region. `None` means the region keeps the quadrant blocks the
/// composer already drew under it, which is always a valid answer.
fn encode(picker: &Picker, frame: &PageFrame, cell: UVec2, tile: Rect) -> Option<Protocol> {
    let (origin, size) = tile_pixels(cell, tile);
    if size.x.checked_mul(size.y)? > MAX_TILE_PIXELS {
        log::debug!("region too large for a graphics upload: {size:?}");
        return None;
    }

    // Copy out the region's rows, so only the region is materialised rather
    // than a second copy of the whole frame.
    let stride = frame.size.x as usize * 4;
    let row_bytes = size.x as usize * 4;
    let mut region = Vec::with_capacity(row_bytes * size.y as usize);
    for row in 0..size.y {
        let start = (origin.y + row) as usize * stride + origin.x as usize * 4;
        region.extend_from_slice(frame.pixels.get(start..start + row_bytes)?);
    }
    let crop = RgbaImage::from_raw(size.x, size.y, region)?;

    let protocol = picker
        .new_protocol(
            DynamicImage::ImageRgba8(crop),
            Size::new(tile.width, tile.height),
            // The crop is already exactly the right number of pixels for the
            // cells it covers, so this never actually resamples.
            Resize::Fit(None),
        )
        .inspect_err(|error| log::debug!("graphics encoding failed, keeping blocks: {error}"))
        .ok()?;

    Some(match protocol {
        Protocol::Sixel(sixel) => Protocol::Sixel(restore_cursor(sixel)),
        other => other,
    })
}

fn tile_pixels(cell: UVec2, tile: Rect) -> (UVec2, UVec2) {
    (
        UVec2::new(u32::from(tile.x) * cell.x, u32::from(tile.y) * cell.y),
        UVec2::new(
            u32::from(tile.width) * cell.x,
            u32::from(tile.height) * cell.y,
        ),
    )
}

/// Bracket a Sixel payload with a cursor save/restore.
///
/// The encoder clears the area it is about to draw into by stepping down one
/// row at a time and then jumping back up. On the last rows of the screen those
/// downward steps clamp, so the jump back overshoots and the image lands too
/// high. Saving the cursor before the clear and restoring it after puts the
/// image where the cell says it should be, wherever on screen that is.
fn restore_cursor(
    mut sixel: ratatui_image::protocol::sixel::Sixel,
) -> ratatui_image::protocol::sixel::Sixel {
    let (start, escape, _) =
        ratatui_image::picker::cap_parser::Parser::tmux_start_escape_end(sixel.is_tmux);
    let introducer = format!("{escape}P");
    let Some(offset) = sixel.data[start.len()..].find(&introducer) else {
        return sixel;
    };
    sixel
        .data
        .insert_str(start.len() + offset, &format!("{escape}8"));
    sixel.data.insert_str(start.len(), &format!("{escape}7"));
    sixel
}
