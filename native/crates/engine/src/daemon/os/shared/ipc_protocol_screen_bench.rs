//! `se screen bench` options as the worker takes them (moved out of
//! `ipc_protocol_screen.rs` for BK0, plan 2026-11-14 „Bildkette“).
use serde::{Deserialize, Serialize};

use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::{MonitorInfo, SourceId};

/// The code of a refused `--source` the host does not have (a monitor id
/// missing from its hello); a source the host refuses itself keeps the
/// host's code.
pub const BENCH_SOURCE_NOT_FOUND: &str = "source_not_found";

/// `se screen bench` options as the worker takes them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchRequest {
    /// `auto`, `h264`, `hevc`, `av1` or `tiles`.
    pub codec: String,
    pub size: FrameSize,
    pub fps: u16,
    pub seconds: u32,
    /// `--source`: `monitor:<id>` or `window:<id>` of the host; absent is
    /// the test picture. A daemon of v0.5.196 ignores the field and measures
    /// the test picture; the client then finds no `source` in the report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `--transport` („Übertragung“, user 2026-11-25); absent is the
    /// default (`realtime`); a daemon before it ignores the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<crate::screen::core::delivery::Delivery>,
}

/// The source a bench request names: the test picture without `--source`.
pub fn bench_source(named: Option<&str>) -> Result<SourceId, String> {
    named.map_or(Ok(SourceId::Synthetic), |text| {
        SourceId::parse(text).map_err(|error| format!("invalid source: {error}"))
    })
}

/// Whether `source` names a monitor the host's hello does not list.
pub fn monitor_missing(source: &SourceId, monitors: &[MonitorInfo]) -> bool {
    matches!(source, SourceId::Monitor(id) if !monitors.iter().any(|monitor| &monitor.id == id))
}

#[cfg(test)]
#[path = "bildkette_bench_source_task_tests.rs"]
mod bildkette_task_tests;
