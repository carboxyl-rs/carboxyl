// that should NOT handle unsafe panics!
// let it core dump, and storngly not recommend debug builds. (maybe see if signal handling can get a code cleanup?)

//! Fatal signal handling for terminal restore.
//!
//! Registers a handler for SIGABRT — an unrecoverable signal that bypasses
//! Rust's panic machinery (e.g. Servo calling abort() on assertion failure).
//!
//! SIGSEGV, SIGBUS, and SIGILL are intentionally excluded: signal-hook forbids
//! them because the process is in undefined state when they arrive. The 64MB
//! Servo thread stack makes stack overflow essentially unreachable in practice (for release builds).
//!
//! Each handler:
//!   1. Writes terminal restore sequences directly to stdout (async-signal-safe).
//!   2. Calls `emulate_default_handler` which resets the disposition and
//!      re-delivers the signal — producing the correct exit status and core dump.
//!
//! # Safety contract
//!
//! All `unsafe` in this module is isolated here by design. The invariants are:
//!
//! - Only async-signal-safe functions are called inside the handler:
//!     * `rustix::io::write` → thin wrapper over `write(2)`.
//!     * `signal_hook::low_level::emulate_default_handler` → resets disposition
//!       and re-delivers the signal atomically.
//! - No heap allocation, no locks, no panicking.
//! - Static byte string only — no runtime formatting.

use std::io;

/// Escape sequences that undo ratatui's terminal initialisation:
///
/// - `\x1b[?25h`   show cursor
/// - `\x1b[?1003l` disable any-event mouse tracking
/// - `\x1b[?1006l` disable SGR mouse encoding
/// - `\x1b[?1049l` exit alternate screen buffer
const RESTORE: &[u8] = b"\x1b[?25h\x1b[?1003l\x1b[?1006l\x1b[?1049l";

// SIGSEGV, SIGBUS, and SIGILL are forbidden by signal-hook — they indicate
// undefined behaviour and signal-hook cannot safely deliver them via its
// handler infrastructure. SIGABRT is allowed and covers Servo abort() calls.
const FATAL_SIGNALS: &[i32] = &[signal_hook::consts::SIGABRT];

/// Register terminal-restore handlers for fatal signals.
///
/// Call once at process startup, before ratatui initialises the terminal.
pub fn register() -> io::Result<()> {
    for &sig in FATAL_SIGNALS {
        // SAFETY: see module-level safety contract.
        unsafe {
            signal_hook::low_level::register(sig, move || {
                // Step 1: restore terminal — async-signal-safe write(2).
                let _ = rustix::io::write(rustix::stdio::stdout(), RESTORE);

                // Emulate the default handler — resets the disposition and
                // re-delivers the signal in one step, avoiding the re-entry
                // hazard of restore + raise. Produces the correct exit status
                // and core dump behaviour.
                let _ = signal_hook::low_level::emulate_default_handler(sig);
            })?;
        }
    }

    Ok(())
}
