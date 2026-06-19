use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// RenderConfig
// ---------------------------------------------------------------------------

/// Immutable rendering parameters derived from CLI flags at startup.
#[derive(Clone, Copy)]
pub struct RenderConfig {
    pub true_color: bool,
    #[cfg(feature = "native-text")]
    pub native_text: bool,
    pub frame_budget: Duration,
}

impl RenderConfig {
    pub fn new(true_color: bool, #[cfg(feature = "native-text")] native_text: bool, fps: u16) -> Self {
        Self {
            true_color,
            #[cfg(feature = "native-text")]
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
#[cfg(feature = "native-text")]
const EXTRACT_DEBOUNCE: Duration = Duration::from_millis(300);

pub struct TimingState {
    last_draw: Instant,
    last_paint_cmd: Instant,
    #[cfg(feature = "native-text")]
    last_extract: Instant,
}

impl TimingState {
    pub fn new(frame_budget: Duration) -> Self {
        let past = Instant::now() - frame_budget;
        Self {
            last_draw: past,
            last_paint_cmd: past,
            #[cfg(feature = "native-text")]
            last_extract: past,
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

    #[cfg(feature = "native-text")]
    pub fn extract_due(&self) -> bool {
        self.last_extract.elapsed() >= EXTRACT_DEBOUNCE
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

    #[cfg(feature = "native-text")]
    pub fn mark_extracted(&mut self) {
        self.last_extract = Instant::now();
    }

    /// Force the next `extract_due` check to return `true`. Called after a
    /// navigation or resize that invalidates the previously extracted text.
    #[cfg(feature = "native-text")]
    pub fn invalidate_extract(&mut self) {
        self.last_extract = Instant::now() - EXTRACT_DEBOUNCE;
    }
}
