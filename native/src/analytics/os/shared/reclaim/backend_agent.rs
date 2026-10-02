//! Reclaim walk through an agent that hashes next to the data (the SSH
//! agent's `WalkHashed`): sizes, times and MD5 arrive in one stream, nothing
//! is downloaded. The stream is all or nothing; a link boundary or a failure
//! inside the tree hands the walk to the listing walk, which reports links
//! and unreadable folders one by one instead of failing the whole search.
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{bounded, RecvTimeoutError};

use super::backend::{record_backend_file, record_limit, BackendAcc, Walk};
use super::budget::ReclaimBudget;
use super::types::{DuplicateEvidence, ReclaimItem, ReclaimProgress};
use super::util::join_path;
use crate::vfs::{Backend, BackendHandle, Scheme};

/// How fast a cancel reaches a running walk even while no entry arrives
/// (one large file being hashed on the server).
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Whether `backend` hashes on the side of the data. The background worker
/// that bridges a Share peer only emulates the stream by downloading every
/// file, so a Share location never walks this way.
pub(super) fn hashes_server_side(backend: &dyn Backend) -> bool {
    backend.supports_walk_hashed() && backend.scheme() != Scheme::Peer
}

/// The walk's counters before it began, restored when the listing walk
/// takes over (nothing is counted twice).
struct Counters {
    files: u64,
    dirs: u64,
    bytes: u64,
}

impl Counters {
    fn take(progress: &ReclaimProgress) -> Self {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        Self {
            files: load(&progress.files),
            dirs: load(&progress.dirs),
            bytes: load(&progress.bytes),
        }
    }

    fn restore(&self, progress: &ReclaimProgress) {
        progress.files.store(self.files, Ordering::Relaxed);
        progress.dirs.store(self.dirs, Ordering::Relaxed);
        progress.bytes.store(self.bytes, Ordering::Relaxed);
    }
}

/// The walk's result, or `None` for the listing walk to take over
/// (unsupported, link boundary, failure inside the tree, worker panic);
/// the listing walk then starts with a fresh budget.
pub(super) fn scan_backend_via_agent(
    backend: &BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    walk: &Walk<'_>,
    budget: &mut ReclaimBudget,
) -> Option<BackendAcc> {
    let counters = Counters::take(progress);
    let (tx, rx) = bounded::<crate::vfs::HashHit>(1024);
    let walk_cancel = AtomicBool::new(false);
    let worker_cancel = &walk_cancel;
    let mut acc = BackendAcc::new();
    let outcome = std::thread::scope(|scope| {
        let worker = scope.spawn(move || backend.walk_hashed(root, true, tx, worker_cancel));
        loop {
            let hit = match rx.recv_timeout(CANCEL_POLL) {
                Ok(hit) => Some(hit),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            // A cancel reaches the walk at once, not only with the next entry.
            if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
                walk_cancel.store(true, Ordering::Relaxed);
                continue;
            }
            let Some(hit) = hit else {
                continue;
            };
            let path = join_path(root, &hit.rel);
            let name = hit
                .rel
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or(hit.rel.as_str())
                .to_string();
            let depth = u32::try_from(
                hit.rel
                    .split('/')
                    .filter(|component| !component.is_empty())
                    .count(),
            )
            .unwrap_or(u32::MAX);
            if let Err(limit) = budget.claim(path.len().saturating_add(name.len()), depth) {
                record_limit(&mut acc, root, limit);
                walk_cancel.store(true, Ordering::Relaxed);
                continue;
            }
            if hit.is_dir {
                progress.dirs.fetch_add(1, Ordering::Relaxed);
                progress.stage.enter_directory(Path::new(&path));
                continue;
            }
            let item = ReclaimItem::new(path, name, hit.size, hit.mtime_ms, false);
            record_backend_file(
                item,
                hit.md5,
                DuplicateEvidence::AgentMd5,
                walk,
                progress,
                true,
                &mut acc,
            );
        }
        worker.join()
    });
    let stopped = budget.stopped() || progress.cancel.load(Ordering::Relaxed);
    match outcome {
        Ok(Ok(true)) => Some(acc),
        // Canceled or at the walk budget: what was seen, with the limit named.
        Ok(_) if stopped => Some(acc),
        // Unsupported before any entry, or started and then failed inside
        // the tree: the listing walk begins again from the root.
        Ok(Ok(false)) | Ok(Err(_)) | Err(_) => {
            counters.restore(progress);
            None
        }
    }
}
