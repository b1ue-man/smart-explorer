//! The process-wide watch service (RV1, contract V4). Contract stage: no
//! operating-system backend is armed yet, so every watch reports
//! `Unavailable(Unsupported)` and consumers keep polling as before. The
//! backends (Linux/Android: one inotify instance for all roots; Windows:
//! `ReadDirectoryChangesW` per root with handle release on volume removal)
//! replace this body without changing the signatures.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::types::{
    UnavailableReason, WatchEvent, WatchFilter, WatchId, WatchMessage, WatchOptions, WatchSink,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One watched root. Dropping it stops the watch; messages already handed to
/// the sink may still arrive afterwards, so consumers ignore unknown ids.
#[derive(Debug)]
pub struct WatchHandle {
    id: WatchId,
    root: PathBuf,
}

impl WatchHandle {
    pub fn id(&self) -> WatchId {
        self.id
    }

    /// The root as passed to `watch`.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Starts watching the tree below `root` (an absolute path). Returns at once:
/// the watch is armed on the service thread and reports `Ready(coverage)` or
/// `Unavailable(reason)` through `sink`, then changes. The app data and cache
/// directories are never watched or reported; links and junctions below the
/// root are never followed. Errors: `InvalidInput` for a relative root.
pub fn watch(
    root: &Path,
    options: WatchOptions,
    filter: WatchFilter,
    sink: WatchSink,
) -> io::Result<WatchHandle> {
    if !root.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "watch root must be an absolute path",
        ));
    }
    let id = WatchId::new(NEXT_ID.fetch_add(1, Ordering::Relaxed));
    // Contract stage: nothing is armed, the consumer polls.
    let _ = (options, filter);
    let _ = sink.deliver(WatchMessage {
        id,
        event: WatchEvent::Unavailable(UnavailableReason::Unsupported),
    });
    Ok(WatchHandle {
        id,
        root: root.to_path_buf(),
    })
}
