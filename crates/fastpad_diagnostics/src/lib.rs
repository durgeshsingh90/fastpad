use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Stopwatch {
    start: Instant,
}

impl Stopwatch {
    pub fn start_new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenMetrics {
    pub file_bytes: u64,
    pub metadata_open: Duration,
    pub first_viewport: Duration,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMetrics {
    pub file_bytes: u64,
    pub bytes_scanned: u64,
    pub matches_seen: u64,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBudget {
    pub perceived_open_10gb_ms: u64,
    pub first_viewport_ms: u64,
    pub scroll_frame_ms: u64,
    pub typing_latency_ms: u64,
    pub first_search_result_ms: u64,
    pub ui_freeze_max_ms: u64,
}

impl Default for RuntimeBudget {
    fn default() -> Self {
        Self {
            perceived_open_10gb_ms: 200,
            first_viewport_ms: 16,
            scroll_frame_ms: 16,
            typing_latency_ms: 8,
            first_search_result_ms: 200,
            ui_freeze_max_ms: 16,
        }
    }
}
