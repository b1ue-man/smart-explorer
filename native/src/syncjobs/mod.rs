//! Persistent sync jobs - the saved "sync setups" (source, target, direction,
//! conflict mode, retention, schedule, hidden/ignore). Stored as one
//! human-readable `key=value` file per job under `<appdata>/smart_explorer/
//! sync/jobs/<id>.conf`, shared by the Sync UI, the split-view "sync these
//! folders" action, and the background worker - so a setup survives a restart
//! and every surface agrees.
//!
//! The `key=value` format is deliberately forward-compatible: unknown keys are
//! ignored and missing keys fall back to defaults, so new options can be added
//! over time without breaking old files or older builds. The previous single
//! positional `jobs.tsv` is auto-imported once on first load.

#[path = "core_oslocked/persistence.rs"]
mod persistence;
#[path = "core_oslocked/results.rs"]
mod results;
#[path = "core/schedule.rs"]
mod schedule;
#[path = "core/types.rs"]
mod types;

#[allow(unused_imports)]
pub use persistence::{jobs_dir, jobs_path, load, remove, save, upsert};
pub use results::{load_results, mark_run, record_result, JobResult};
#[allow(unused_imports)]
pub use schedule::within_window;
pub use types::{SyncJob, Trigger};
