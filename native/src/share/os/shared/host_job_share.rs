//! The share of the cores one host analysis job is counted with when the
//! host admits concurrent jobs (`analysis_admission::host`), on the desktop.
//!
//! It stays the two threads a desktop analysis used until 2026-10-31
//! (`SMART_EXPLORER_ANALYTICS_THREADS` still overriding it, clamped to the
//! cores, as `analytics::local_scan_threads` did). The analysis now lists
//! with every core (`analytics::local_scan_threads`); dividing the cores by
//! that would admit a single host job on every machine, so a second device's
//! analysis or sync walk would wait behind the first one. Several jobs on
//! all cores share them, as two jobs on two threads each did before.
pub(super) fn host_job_share() -> usize {
    let cores = std::thread::available_parallelism().map_or(2, |cores| cores.get());
    std::env::var("SMART_EXPLORER_ANALYTICS_THREADS")
        .ok()
        .and_then(|threads| threads.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, cores.max(1))
}
