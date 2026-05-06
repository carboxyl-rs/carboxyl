//! Fatal signal handling for terminal restore.
//!
//! Registers a handler for [`SIGABRT`] — a signal that indicates an
//! unrecoverable error (e.g. Servo calling `abort()` on an assertion failure)
//! and that bypasses Rust's panic hook machinery.
//!
//! The handler restores the terminal before the process dies, so the shell
//! is not left in raw/alt-screen mode after a crash.
//!
//! ## Why only SIGABRT?
//!
//! `signal-hook` explicitly forbids registering handlers for `SIGSEGV`,
//! `SIGBUS`, and `SIGILL` because those signals indicate that the *process
//! itself* is in an undefined state — the memory that backs your handler's
//! stack frame may already be corrupt. There is no safe way to handle them.
//! The 64 MB Servo thread stack makes stack overflow essentially unreachable
//! in release builds; debug-mode crashes are accepted as-is.
//!
//! ## Safety
//!
//! All `unsafe` in this module is isolated here by design. The invariants:
//!
//! - Only async-signal-safe operations are called inside the handler:
//!   - [`rustix::io::write`] is a direct `write(2)` syscall with no
//!     allocation, no locking, and no errno-clobbering side effects.
//!     POSIX lists `write` as async-signal-safe (POSIX.1-2017 §2.4.3).
//!   - [`signal_hook::low_level::emulate_default_handler`] resets the
//!     signal disposition via `sigaction` and re-raises the signal atomically,
//!     producing the correct exit status and core-dump behaviour.
//! - The `RESTORE` constant is `&'static [u8]` — no heap allocation occurs.
//! - No locks, no panicking, no formatting inside the handler.
//! - The signal disposition is reset before re-delivery, preventing
//!   re-entrance into this handler.

use std::io;

/// Raw escape sequences that undo ratatui's terminal initialisation:
///
/// | Sequence       | Effect                          |
/// |----------------|---------------------------------|
/// | `\x1b[?25h`   | Show cursor (ratatui hides it)  |
/// | `\x1b[?1003l` | Disable any-event mouse tracking |
/// | `\x1b[?1006l` | Disable SGR mouse encoding      |
/// | `\x1b[?1049l` | Exit alternate screen buffer    |
const RESTORE: &[u8] = b"\x1b[?25h\x1b[?1003l\x1b[?1006l\x1b[?1049l";

/// Register a terminal-restore handler for [`SIGABRT`].
///
/// Call once at process startup, before ratatui initialises the terminal.
/// Safe to call multiple times — subsequent registrations stack; the most
/// recently registered handler runs first.
///
/// # Errors
///
/// Returns an error if the OS rejects the `sigaction` call (extremely rare).
pub fn register() -> io::Result<()> {
    // SAFETY: see module-level safety contract above.
    unsafe {
        signal_hook::low_level::register(signal_hook::consts::SIGABRT, || {
            // Step 1: restore terminal — direct write(2), async-signal-safe.
            let _ = rustix::io::write(rustix::stdio::stdout(), RESTORE);
            // Step 2: reset disposition and re-deliver with default behaviour
            // (core dump / exit status). emulate_default_handler is atomic:
            // it unregisters this handler before raising, preventing re-entry.
            let _ = signal_hook::low_level::emulate_default_handler(signal_hook::consts::SIGABRT);
        })?;
    }

    Ok(())
}
