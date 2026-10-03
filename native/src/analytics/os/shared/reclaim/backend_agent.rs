//! Reclaim walk with digests computed next to the data (the SSH agent, a
//! Share host with `hash_walk_v1`): sizes, times and MD5 arrive in one
//! stream, nothing is downloaded. Links and special files are left out,
//! unreadable entries are reported one by one; a walk that fails as a whole
//! hands over to the listing walk, which starts again from the root.
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{bounded, RecvTimeoutError};

use super::backend::{record_backend_file, record_limit, BackendAcc, Walk};
use super::budget::ReclaimBudget;
use super::types::{DuplicateEvidence, HashAlgorithm, ReclaimItem, ReclaimProgress};
use super::util::join_path;
use crate::vfs::{BackendHandle, HashWalkItem, HashWalkRequest, OmissionReason};

/// How fast a cancel reaches a running walk even while no entry arrives
/// (one large file being hashed on the server).
const CANCEL_POLL: Duration = Duration::from_millis(100);

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

/// The walk's result, or `None` for the listing walk to take over (the
/// walk is unsupported or failed as a whole; it then starts with a fresh
/// budget). Files below `min_bytes` are left out by the serving side.
pub(super) fn scan_backend_hash_walk(
    backend: &BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    walk: &Walk<'_>,
    min_bytes: u64,
    budget: &mut ReclaimBudget,
) -> Option<BackendAcc> {
    let counters = Counters::take(progress);
    let (tx, rx) = bounded::<HashWalkItem>(1024);
    let walk_cancel = AtomicBool::new(false);
    let worker_cancel = &walk_cancel;
    let request = HashWalkRequest {
        algorithm: Some(HashAlgorithm::Md5),
        min_bytes,
    };
    let mut acc = BackendAcc::new();
    let outcome = std::thread::scope(|scope| {
        let worker = scope
            .spawn(move || crate::vfs::hash_walk(&**backend, root, request, tx, worker_cancel));
        loop {
            let item = match rx.recv_timeout(CANCEL_POLL) {
                Ok(item) => Some(item),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            // A cancel reaches the walk at once, not only with the next entry.
            if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
                walk_cancel.store(true, Ordering::Relaxed);
                continue;
            }
            match item {
                Some(HashWalkItem::Entry(entry)) => {
                    let path = join_path(root, &entry.rel);
                    let name = entry
                        .rel
                        .rsplit('/')
                        .next()
                        .filter(|name| !name.is_empty())
                        .unwrap_or(entry.rel.as_str())
                        .to_string();
                    let depth = u32::try_from(
                        entry
                            .rel
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
                    if entry.is_dir {
                        progress.dirs.fetch_add(1, Ordering::Relaxed);
                        progress.stage.enter_directory(Path::new(&path));
                        continue;
                    }
                    let item = ReclaimItem::new(path, name, entry.size, entry.mtime_ms, false);
                    record_backend_file(
                        item,
                        entry.digest,
                        DuplicateEvidence::AgentMd5,
                        walk,
                        progress,
                        true,
                        &mut acc,
                    );
                }
                Some(HashWalkItem::Omitted(omitted)) => match omitted.reason {
                    // Links are walk boundaries and special files hold no
                    // content: neither can be a duplicate.
                    OmissionReason::Link | OmissionReason::Special => {}
                    _ => acc.error(format!(
                        "{}: {}",
                        join_path(root, &omitted.rel),
                        omitted.detail
                    )),
                },
                None => {}
            }
        }
        worker.join()
    });
    let stopped = budget.stopped() || progress.cancel.load(Ordering::Relaxed);
    match outcome {
        Ok(Ok(true)) => Some(acc),
        // Canceled or at the walk budget: what was seen, with the limit named.
        Ok(_) if stopped => Some(acc),
        // Unsupported before any entry, or failed as a whole (transport, the
        // root): the listing walk begins again from the root.
        Ok(Ok(false)) | Ok(Err(_)) | Err(_) => {
            counters.restore(progress);
            None
        }
    }
}
