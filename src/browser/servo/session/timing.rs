use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// RenderConfig
// ---------------------------------------------------------------------------

/// Immutable rendering parameters derived from CLI flags at startup.
#[derive(Clone, Copy)]
pub struct RenderConfig {
    pub true_color: bool,
    pub frame_budget: Duration,
}

impl RenderConfig {
    pub fn new(true_color: bool, fps: u16) -> Self {
        Self {
            true_color,
            frame_budget: Duration::from_millis(1000 / fps.max(1) as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// TimingState
// ---------------------------------------------------------------------------

/// Tracks when the last draw and paint command occurred so frame-rate
/// limiting is expressed in one place rather than scattered `Instant`s
/// throughout the loop.
pub struct TimingState {
    last_draw: Instant,
    last_paint_cmd: Instant,
}

impl TimingState {
    pub fn new(frame_budget: Duration) -> Self {
        let past = Instant::now() - frame_budget;
        Self {
            last_draw: past,
            last_paint_cmd: past,
        }
    }

    pub fn draw_due(&self, budget: Duration) -> bool {
        self.last_draw.elapsed() >= budget
    }

    pub fn paint_cmd_due(&self, budget: Duration) -> bool {
        self.last_paint_cmd.elapsed() >= budget
    }

    pub fn mark_drawn(&mut self) {
        self.last_draw = Instant::now();
    }

    pub fn mark_paint_cmd(&mut self) {
        self.last_paint_cmd = Instant::now();
    }
}
