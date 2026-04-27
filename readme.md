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

The project keeps the existing terminal UI, input parser, navigation bar, and cell renderer, but the browser engine is now embedded directly from Rust through Servo's `WebView` API and `SoftwareRenderingContext`.

## Status

- The active runtime path is Servo-based.
- Chromium-specific build glue and launch scripts are removed from the supported workflow.
- Rendering currently goes through Servo's software rendering context so it can stay terminal-first and window-server-free.

## Usage

```console
$ cargo run --release -- https://example.com
```

Or after building:

```console
$ ./target/release/carboxyl https://example.com
```

## Build

```console
$ cargo build --release
```

The build is now a normal Cargo build for the `carboxyl` binary. The first build will download the published `servo` crate and its dependencies through Cargo.

## Runtime Architecture

- `src/browser/servo_runtime.rs` owns the browser runtime.
- Servo is embedded through `ServoBuilder`, `WebViewBuilder`, and `SoftwareRenderingContext`.
- Terminal input is translated directly into Servo input events.
- The existing Rust renderer still paints the terminal viewport and navigation UI.

## Notes

- Servo evolves quickly. If Cargo resolution or toolchain requirements drift, align the local toolchain with upstream Servo and refresh the lockfile.
- Legacy Chromium files may still exist in older branches or release artifacts, but they are no longer part of the supported runtime path in this branch.
