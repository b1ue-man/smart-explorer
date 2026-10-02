//! Change watching for sync roots and Share exports (RV1, contract V4).
//!
//! One process-wide service: Linux and Android use a single inotify instance
//! for every root (one watch per directory, links never followed, other file
//! systems below the root skipped unless `cross_mounts`); Windows uses
//! `ReadDirectoryChangesW` per root (large buffer locally, at most 64 KiB on
//! network paths) and closes the handle when its volume is being removed, so
//! safe removal works, then arms it again on arrival. Lost events become
//! `Overflow`; a root that cannot be watched becomes `Unavailable(reason)`.
//!
//! Events are triggers only: what changed is decided by whoever rescans (the
//! sync engine, the Share host). The app's own data and cache directories are
//! never watched or reported. Consumers: the background worker (real-time
//! jobs) and the Share host (`watch_v1`); the Android host feeds MediaStore
//! signals through `report_host_change` and `set_host_cursor`.

#[path = "os/shared/host_signal.rs"]
mod host_signal;
#[path = "os/shared/service.rs"]
mod service;
#[path = "core/types.rs"]
mod types;

pub use host_signal::{host_cursor, report_host_change, set_host_cursor};
pub use service::{watch, WatchHandle};
pub use types::{
    Change, Coverage, EventKind, UnavailableReason, WatchEntry, WatchEvent, WatchFilter, WatchId,
    WatchMessage, WatchOptions, WatchSink,
};
