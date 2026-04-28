# TODO

* Optional hi-res terminal graphics (Sixel / Kitty)

  * Feature detection + ANSI fallback

* Text rendering using terminal native text

* Implement Standarized GitHub CI/CD/Workflow

* Servo crashes under load (looks like to be a more a problem on debug builds)

  * Stack overflow + address boundary issues
  * Reproduce, log, open upstream issue

* Panic handling is weak

  * Use `set_hook`
  * Prevent terminal corruption

* Signal handling

  * Use `signal-hook` (SIGINT, SIGTERM)
  * Route to graceful shutdown

* Terminal restore (ratatui)

  * Must restore on panic / signal / exit

* Centralize graceful exit

  * Cleanup, restore, flush logs

* Docker image

  * Create reproducible build environment
  * Optimize image size (multi-stage build)
  * Ensure proper terminal support inside container

---

# Docs / README Improvements

* Document terminal graphics support

  * Supported protocols (Sixel / Kitty; when ready)
  * Fallback behavior

* Document Servo instability

  * Known crash scenarios
  * Current limitations
  * Link to upstream issue in Servo once created
    
* Document Docker usage

---
