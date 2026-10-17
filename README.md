<table align="center">
  <tbody>
    <tr>
      <td>
        <pre>
      O
    //
R — C
      \
       OH</pre>
      </td>
      <td><h1>Carboxyl</h1></td>
    </tr>
  </tbody>
</table>

Carboxyl is a community fork of [Carbonyl](https://github.com/fathyb/carbonyl), now rebuilt around [Servo](https://github.com/servo/servo) instead of a patched Chromium runtime.

It renders web pages in your terminal, with a configurable frame-rate limit of 60 FPS by default.
It does not require a window server (i.e. works in a safe-mode console), and even runs through SSH.

> Join the Carboxyl community on [Discord](https://discord.gg/jzun3aN2ak).
>
> <a href="https://discord.gg/jzun3aN2ak">
>  <img alt="Join the Discord server" src="https://img.shields.io/badge/Discord-Join%20server-5865F2?logo=discord&logoColor=white">
> </a>

## Status

> Carboxyl now has a dedicated build server, which runs its CI and release builds, provided by [Lysator](https://www.lysator.liu.se/)!
> See [this issue](https://github.com/carboxyl-rs/carboxyl/issues/9) for details.

- The active runtime path is Servo-based.
- Chromium-specific build glue and launch scripts are removed from the supported workflow.
- Rendering goes through Servo's software rendering context so it can stay terminal-first and window-server-free.
- Servo 0.7 is resolved entirely from crates.io, including its component crates.
- Native terminal text comes from Servo's accessibility tree, which Servo pushes on every load, relayout and scroll. Text and pixels are produced in the same Servo turn, so they cannot disagree about where the page is. See [how rendering works](docs/rendering.md).

## Usage

```console
$ cargo run --release -- https://example.com
```

Or after building:

```console
$ ./target/release/carboxyl https://example.com
```

Text is drawn with the terminal's own glyphs, flat backgrounds become real
terminal colors, and only the cells with actual detail in them - images,
gradients, canvases, form controls - are drawn as pixels. `--no-native-text`
turns the glyph layer off and renders everything as pixels instead.

`--scale` is Servo's page zoom, so it behaves like a desktop browser's: higher
zooms in, lower shows more of the page.

Detail cells use Kitty, Sixel or iTerm2 graphics when the terminal has them, and
quadrant blocks when it does not. `--graphics auto|kitty|sixel|quadrant` picks a
mode; anything unsupported falls back to quadrant blocks. Each region is encoded
once and cached by content, so scrolling re-places uploads the terminal already
holds rather than re-sending them.

For all-pixel output with no terminal graphics at all:

```console
$ carboxyl --graphics quadrant --no-native-text https://example.com
```

Bundled resources, the JavaScript JIT, WebCrypto, WebGL, Brotli compression
streams, and multiprocess support are included. Desktop clipboard, WebGPU,
WebXR, and GStreamer media are not enabled in the default terminal build.

## Build

```console
$ cargo build --release --locked
```

The build is now a normal Cargo build for the `carboxyl` binary. The first build will download the published `servo` crate and its dependencies through Cargo.

## Validation

```console
$ cargo test --locked
$ cargo clippy --locked --workspace --all-targets -- -D warnings
$ cargo run --release --locked --example render_smoke -- target/render-smoke
```

Composition is a pure function of a page frame, so the cases that are easy to get
wrong - occlusion, fixed headers, scrolled boxes, wrapping, wide graphemes,
inline layout, graphics tiling - are covered by ordinary unit tests in
`cargo test`, with no Servo, no network and no window server.

`render_smoke` is the end-to-end check: it drives real Servo layout and software
rendering over `tests/fixtures/rendering.html`, which is built to exercise those
same cases, then scrolls, resizes and navigates away. It writes a text snapshot
of each screen to `target/render-smoke`.

## Runtime Architecture

- `browser::servo` owns the browser runtime and is the only place Servo's types appear.
- Servo is embedded through `ServoBuilder`, `WebViewBuilder`, and `SoftwareRenderingContext`.
- `browser::servo::servo_thread` produces one `PageFrame` per composite: Servo's pixels plus the text painted into them.
- `output::compose` turns a `PageFrame` into terminal cells, and `output::graphics` draws the few regions that need real pixels.
- Terminal input (by crossterm) is translated directly into Servo input events.
- Output is being handled with ratatui.

## TODO

* Upstream text-run bounds ([servo#47164](https://github.com/servo/servo/issues/47164)) so line breaks can follow Servo's own instead of being re-derived

* A fancy browser ui and more features (history, cookies*, tabs, etc)

* Optional hi-res terminal graphics (Sixel / Kitty)

  * Feature detection + ANSI fallback

## Notes

- While this is in the 'servo-dev' branch, discuss on #16.
- Please build only with --release, otherwise you may get panics on runtime. Release builds might take some time, but way less than compiling chromium.
- Servo evolves quickly. If Cargo resolution or toolchain requirements drift, align the local toolchain with upstream Servo and refresh the lockfile.
- Legacy Chromium files may still exist in older branches or release artifacts, but they are no longer part of the supported runtime path in this branch.
