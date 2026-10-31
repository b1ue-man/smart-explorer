//! Folder-by-folder walk of a remote root without level barriers. Listing
//! threads take the next pending folder from one shared queue as soon as they
//! are free (work stealing). The driver thread alone validates, budgets and
//! emits, so the limits and the emitted rows are those of `WalkState`. A slow
//! listing delays only its own folder; the former barrier made each block of
//! `parallelism` folders wait for its slowest listing before the next block.
use super::walk_state::{PendingRemoteDir, WalkState};
use crate::vfs::{BackendHandle, VfsMeta};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// More concurrent listings than this never sped up one remote connection.
const MAX_LISTERS: usize = 16;

/// One finished listing: its folder, and the entries or the error text.
type Listed = (PendingRemoteDir, Result<Vec<VfsMeta>, String>);

/// Walks `root` folder by folder with work stealing. The width is the
/// backend's parallelism clamped to `2..=16`, as before.
pub(super) fn run(backend: BackendHandle, root: String, mut state: WalkState) {
    let width = backend.parallelism().clamp(2, MAX_LISTERS);
    let (jobs_tx, jobs_rx) = unbounded::<PendingRemoteDir>();
    let (done_tx, done_rx) = unbounded::<Listed>();
    let started = std::thread::scope(|scope| {
        let mut started = 0usize;
        for index in 0..width {
            let lister = Lister {
                backend: backend.clone(),
                jobs: jobs_rx.clone(),
                done: done_tx.clone(),
                cancel: state.cancel_handle(),
            };
            let spawned = std::thread::Builder::new()
                .name(format!("rscan-list-{index}"))
                .spawn_scoped(scope, move || lister.run());
            if spawned.is_err() {
                break;
            }
            started += 1;
        }
        // Only the listing threads hold these ends now. When they all stop,
        // the driver sees the closed results channel instead of waiting.
        drop(jobs_rx);
        drop(done_tx);
        if started > 0 {
            drive(&mut state, root.clone(), jobs_tx, done_rx);
        }
        started
    });
    if started == 0 {
        serial_fallback(backend, root, state);
    } else {
        state.finish();
    }
}

/// Queues the root and every folder a listing reveals, and processes the
/// listings as they finish, until nothing is in flight or the walk stops.
/// Dropping `jobs` on return closes the queue, so the listing threads end.
fn drive(
    state: &mut WalkState,
    root: String,
    jobs: Sender<PendingRemoteDir>,
    done: Receiver<Listed>,
) {
    let mut in_flight = 0usize;
    if jobs.send(PendingRemoteDir::root(root)).is_ok() {
        in_flight += 1;
    }
    while in_flight > 0 && !state.stopped() {
        let Ok((directory, listed)) = done.recv() else {
            break;
        };
        in_flight -= 1;
        match listed {
            Ok(entries) => {
                let mut next = Vec::new();
                let open = state.process_listing(&directory, entries, true, &mut next);
                for child in next {
                    if jobs.send(child).is_ok() {
                        in_flight += 1;
                    }
                }
                if !open {
                    break;
                }
            }
            Err(error) => state.listing_failed(&directory.path, error),
        }
        if !state.maybe_progress(&directory.path) {
            break;
        }
    }
}

/// One listing thread: lists each folder it takes from the queue. After a
/// stop, the folders still queued are answered with an empty listing at once.
struct Lister {
    backend: BackendHandle,
    jobs: Receiver<PendingRemoteDir>,
    done: Sender<Listed>,
    cancel: Arc<AtomicBool>,
}

impl Lister {
    fn run(self) {
        while let Ok(directory) = self.jobs.recv() {
            let listed = if self.cancel.load(Ordering::Relaxed) {
                Ok(Vec::new())
            } else {
                self.backend
                    .list_dir(&directory.path)
                    .map_err(|error| error.to_string())
            };
            if self.done.send((directory, listed)).is_err() {
                return;
            }
        }
    }
}

fn serial_fallback(backend: BackendHandle, root: String, mut state: WalkState) {
    let mut frontier = vec![PendingRemoteDir::root(root)];
    while let Some(directory) = frontier.pop() {
        if state.stopped() {
            break;
        }
        match backend.list_dir(&directory.path) {
            Ok(entries) => {
                if !state.process_listing(&directory, entries, true, &mut frontier) {
                    break;
                }
            }
            Err(error) => state.listing_failed(&directory.path, error),
        }
        if !state.maybe_progress(&directory.path) {
            break;
        }
    }
    state.finish();
}
