//! The share of the cores one host analysis job is counted with when the
//! host admits concurrent jobs (`analysis_admission::host`), on Android: the
//! threads of one local analysis there (`analytics::local_scan_threads`, at
//! most four), unchanged by the desktop's switch to every core (2026-10-31).
pub(super) fn host_job_share() -> usize {
    crate::analytics::local_scan_threads()
}
