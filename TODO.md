# TODO

* Optional hi-res terminal graphics (Sixel / Kitty)

  * Feature detection + ANSI fallback

* Servo crashes under load (google.com)

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

* Investigate Servo instability further
