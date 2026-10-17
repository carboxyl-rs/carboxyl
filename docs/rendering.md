# How Carboxyl renders a page

Carboxyl draws web pages with the terminal's own glyphs wherever it can, and
with pixels where it must. This is how that works, and where it stops working.

## One frame, one source of truth

Everything the renderer needs arrives together, in one `PageFrame`:

```rust
pub struct PageFrame {
    pub size: UVec2,            // viewport, device pixels
    pub pixels: Vec<u8>,        // what Servo painted
    pub text: Arc<[TextBox]>,   // the text it painted, and where
}
```

Both halves are produced in the same Servo turn, by `servo_thread::paint`. That
is the property the rest of the design rests on: the glyph layer and the pixel
layer use the same frame, with no second clock to drift against. Scrolling,
resizing and navigating are all new frames. Servo can still apply a root scroll
transform to fixed elements in its accessibility tree. The compositor retains
the previous raster to detect this: a moved text box keeps its previous position
only when its entire old rectangle is visible and every pixel there is unchanged.
Intersecting scrolled lines stay in the pixel layer, so they cannot overwrite
the fixed text. Changed pixels, resizes and navigation discard that evidence.

## Where the text comes from

`WebView::set_accessibility_active(true)` makes Servo push an
`accesskit::TreeUpdate` whenever anything that affects the tree changes - a load,
a DOM mutation, a relayout, a scroll, a zoom. `browser::servo::a11y` feeds those
into an [`accesskit_consumer::Tree`] and reads the text back out of it, with
geometry already composed through the scroll and device-pixel transforms. No
JavaScript, no polling, no DOM mutation, and no round trip: by the time a frame
is painted, the text for that frame is already in hand.

What the tree does not provide ([servo/servo#47164]) is geometry for text nodes
themselves - only for elements. A `Role::TextRun` is emitted per DOM text node,
not per line box, and carries no bounds. So each run is anchored to the box of
its nearest element, and `output::compose` lays the run out inside that box
against the terminal grid.

Two things make that behave:

- **Inline runs stay together.** Links, emphasis and other phrasing content join
  their enclosing box in document order. This avoids splitting proportional
  inline geometry into incompatible terminal columns. Nested block boxes still
  reserve their cells before their ancestors, ordered by tree depth.
- **The pixels decide where the lines are.** `Pixels::text_rows` finds the bands
  of ink inside a box: one per line, with the cell its leftmost ink falls in.
  When there are as many bands as wrapped lines, each line goes on its own band,
  at its own starting column - so padding, indentation and `text-align` come
  from the render rather than being guessed, which matters because the
  accessibility tree reports border boxes and says nothing about where inside
  them the text sits. Otherwise the lines are spread evenly down the box, which
  is where a browser puts them.

## Where the colors and the occlusion come from

Every cell is described by one pass over the pixels Servo painted into it, which
yields its dominant color (the background), the most contrasting color in it (the
ink) and the four sub-cell averages (for quadrant blocks).

Taking colors from the raster rather than from CSS means dark mode, gradients,
link colors, `currentColor` and inherited colors are right for free - the
accessibility tree exposes none of them.

It also settles occlusion, which the accessibility tree does not expose at all:
it has no stacking order. The render is the only witness, and it is consulted
twice:

- A box with no ink anywhere is skipped entirely. That covers `opacity: 0`,
  `visibility: hidden`, a `clip-path`, text painted in its own background color,
  and a box completely covered by something opaque.
- Within a line, a glyph is drawn only into a cell that still shows the same
  paint as the cell the line starts in - its paper or its pen
  (`Cell::belongs_with`). Blank paper after the last word matches, so a line is
  never clipped just because the terminal's monospace rendering ran a little
  past the page's own ink. A cell now showing a fixed header, a dropdown or a
  modal does not match, and is left to the pixel layer.

Together those handle cases paint-order reasoning would get wrong, without ever
needing to know which element is on top.

## What each cell becomes

`output::compose` gives every cell exactly one of three forms:

1. a **native glyph**, where a text box puts a character and the cell has ink;
2. a **styled space**, where one solid color describes the cell - most of a page,
   and both cheaper and sharper than approximating a flat background with blocks;
3. a **quadrant block**, where the cell has real detail in it.

Only the third kind is reported to the graphics layer, merged into as few
rectangles as possible. On a page of text that is nearly nothing, which is why
images keep up: `output::graphics` encodes each region once and caches it by the
*content* of its pixels, so scrolling an image re-places an upload the terminal
already holds instead of re-encoding and re-transmitting it. Two copies of the
same image share one upload. Regions too small to be worth an upload keep the
quadrant blocks already drawn underneath them, which is also the fallback when a
terminal has no graphics protocol, when encoding fails, and while a page is still
settling.

## Geometry

The viewport is sized to exactly `cells * cell_size` device pixels, where
`cell_size` is the terminal's own cell size - from the graphics query, else from
`TIOCGWINSZ`, else 8x16. Two things follow:

- every cell maps to a whole block of device pixels, so a click lands on the
  pixels its cell shows, with no rounding (`geometry::viewport_point`), and
- one device pixel is one terminal pixel, so a graphics tile is a straight crop
  of Servo's buffer with no resampling.

Zoom is Servo's own page zoom, not a change to this mapping, so `--scale` makes
the page larger or smaller inside a fixed viewport exactly as a desktop browser
does. At the default zoom a page's 16px text comes to roughly one character per
cell, which is what keeps wrapped text from drifting against its box.

## Limits

- **Line breaks are ours, not Servo's.** Without per-line geometry, a long
  paragraph can wrap to one line more or fewer than the page did. The error stays
  inside the box - a box's text is clipped to its own box, and never shifts
  another box's.
- **Preformatted text is inferred.** `<pre>` has no accessibility role, so text
  is laid out both collapsed and as written, and the reading whose line count
  matches the ink bands wins. A code block in a box whose bands cannot be
  separated reads as prose.
- **Text over a photo** loses its glyphs where the photo's colors do not match
  the line's own paint, and falls back to the pixel layer - which still shows
  that text, as pixels. Carboxyl prefers that to drawing a line of glyphs over
  an image the page may have put on top of it.
- **Boxes with a border on the side** can have every row read as inked, which
  collapses their lines into one band; the even spread takes over.
- **Form controls, canvas, SVG, video and WebGL** have no text in the tree and
  stay pixels, as do generated CSS content and anything in a shadow tree Servo
  does not expose.
- `--no-native-text` turns the glyph layer off entirely and renders Servo's
  output as pixels, which is the right answer when these details matter.

## Dependency

`servo = "0.7"`, resolved with all its component crates from crates.io. The
terminal build keeps bundled resources, the JavaScript JIT, WebCrypto, WebGL,
Brotli compression streams and multiprocess; it leaves out the desktop clipboard
(a terminal or SSH session has none), WebGPU, WebXR and GStreamer media.

Servo pins RSA to `0.10.0-rc.18`, which needs PKCS#1 `0.8.0-rc.4`; `rc.5` changes
its key types and fails to compile. The lockfile holds the working version and
every build command passes `--locked`.

[`accesskit_consumer::Tree`]: https://docs.rs/accesskit_consumer
[servo/servo#47164]: https://github.com/servo/servo/issues/47164
