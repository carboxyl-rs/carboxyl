use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// RenderConfig
// ---------------------------------------------------------------------------

/// Immutable rendering parameters derived from CLI flags at startup.
///
/// Passed by shared reference so we never thread `&Cli` or bare bools through
/// function signatures.
#[derive(Clone, Copy)]
pub struct RenderConfig {
    pub true_color: bool,
    pub native_text: bool,
    pub frame_budget: Duration,
}

impl RenderConfig {
    pub fn new(true_color: bool, native_text: bool, fps: u16) -> Self {
        Self {
            true_color,
            native_text,
            frame_budget: Duration::from_millis(1000 / fps.max(1) as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// TimingState
// ---------------------------------------------------------------------------

/// Tracks when the last draw, paint command, and text-extract occurred so
/// debounce / frame-rate limiting is expressed in one place rather than
/// scattered `Instant`s throughout the loop.
pub struct TimingState {
    last_draw: Instant,
    last_paint_cmd: Instant,
    last_extract: Instant,
    extract_debounce: Duration,
}

impl TimingState {
    pub fn new(frame_budget: Duration) -> Self {
        // Initialise all timestamps in the past so the first event fires
        // immediately rather than waiting a full budget cycle.
        let past = Instant::now() - frame_budget;
        Self {
            last_draw: past,
            last_paint_cmd: past,
            last_extract: past,
            extract_debounce: Duration::from_millis(300),
        }
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn draw_due(&self, budget: Duration) -> bool {
        self.last_draw.elapsed() >= budget
    }

    pub fn paint_cmd_due(&self, budget: Duration) -> bool {
        self.last_paint_cmd.elapsed() >= budget
    }

    pub fn extract_due(&self) -> bool {
        self.last_extract.elapsed() >= self.extract_debounce
    }

    // ------------------------------------------------------------------
    // Mutations
    // ------------------------------------------------------------------

    pub fn mark_drawn(&mut self) {
        self.last_draw = Instant::now();
    }

    pub fn mark_paint_cmd(&mut self) {
        self.last_paint_cmd = Instant::now();
    }

    pub fn mark_extracted(&mut self) {
        self.last_extract = Instant::now();
    }

    /// Force the next `extract_due` check to return `true`. Called after a
    /// navigation or resize that invalidates the previously extracted text.
    pub fn invalidate_extract(&mut self) {
        self.last_extract = Instant::now() - self.extract_debounce;
    }
}
