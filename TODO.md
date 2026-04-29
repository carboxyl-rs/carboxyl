# TODO

* Optional hi-res terminal graphics (Sixel / Kitty)

  * Feature detection + ANSI fallback

* Text rendering using terminal native text
* 
* Handle unsafe panics like SIGSEGV, SIGBUS, SIGABRT, SIGILL (on a separate module)

* Fix Enter bug (kind of known on crossterm): enter actually types 'm' into the browser

* Docker image

  * Create reproducible build environment
  * Optimize image size (multi-stage build)
  * Ensure proper terminal support inside container

---

# Docs / README Improvements

* Document terminal graphics support

  * Supported protocols (Sixel / Kitty; when ready)
  * Fallback behavior
  
* Document Docker usage

---
