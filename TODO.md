# TODO

* Docker image

  * Create reproducible build environment
  * Optimize image size (multi-stage build)
  * Ensure proper terminal support inside container

* Text rendering using terminal native text (IN PROGRESS)

  * Try not using servo to render space with text but rendering terminal text instead;
  * Align native text correctly, test it in all and extreme '--resolution'

* Main keys are working but it'd be better if everything were sent to servo; like ctrl-letter, shift, etc.

* A fancy browser ui (as ratatui is already implemented)

* Optional hi-res terminal graphics (Sixel / Kitty)

  * Feature detection + ANSI fallback

---

# Docs / README Improvements

* Document terminal graphics support (when added)

  * Supported protocols (Sixel / Kitty; when ready)
  * Fallback behavior
  
* Document Docker usage (when added)

---
