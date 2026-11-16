//! Browsing handles and jobs (plan 2026-11-14, E4): a handle a person browses
//! a Share device with has the origin `User` (`open_share_backend_for`,
//! `open_share_in_process`), so its listings and status queries set metadata
//! presence. A transfer job of the app that runs in this process (when the
//! worker does not take it, or on Android, where the embedded worker serves
//! this process) used to run over that same handle and made its own
//! transfers yield to its own listings. `job_sides` opens a handle of the
//! origin `Job` for such a side; `transfer::run_view` runs the job over it.
//! The Explorer view keeps its handle and stays `User`.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use crate::share::{OperationOrigin, PeerOpenTarget};
use crate::transfer::{JobSides, JobView};
use crate::vfs::{Backend, BackendHandle};

/// Live handles of the origin `User` (with their thin data address) and the
/// device each reaches.
type Handles = Vec<(Weak<dyn Backend>, usize, PeerOpenTarget)>;

static USER_HANDLES: Mutex<Handles> = Mutex::new(Vec::new());

fn handles() -> MutexGuard<'static, Handles> {
    USER_HANDLES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Remembers `handle` when it was opened for `origin = User`; entries end
/// with their handle. Handles of the origin `Job` are never listed.
pub(super) fn remember_origin(
    handle: &BackendHandle,
    target: &PeerOpenTarget,
    origin: OperationOrigin,
) {
    if origin != OperationOrigin::User {
        return;
    }
    let mut handles = handles();
    handles.retain(|(backend, _, _)| backend.strong_count() > 0);
    let address = crate::transfer::address_of(&**handle);
    handles.push((Arc::downgrade(handle), address, target.clone()));
}

fn find(backend: &dyn Backend) -> Option<PeerOpenTarget> {
    let address = crate::transfer::address_of(backend);
    handles()
        .iter()
        .find(|(live, at, _)| live.strong_count() > 0 && *at == address)
        .map(|(_, _, target)| target.clone())
}

/// The device a handle of the origin `User` reaches, also through a browsing
/// cache; `None` for every other backend.
fn user_target(backend: &dyn Backend) -> Option<PeerOpenTarget> {
    find(backend).or_else(|| find(&*backend.uncached_backend()?))
}

/// The handles a job of this process uses (`TransferWorker::job_sides`).
pub(super) fn job_sides(view: &JobView<'_>) -> JobSides {
    job_sides_with(view, &open_for_job)
}

/// [`job_sides`] with the way a handle is opened named by the caller.
pub(super) fn job_sides_with(
    view: &JobView<'_>,
    open: &dyn Fn(&PeerOpenTarget) -> Option<BackendHandle>,
) -> JobSides {
    let source = view.source.backend().and_then(user_target);
    let target = view.target.backend().and_then(user_target);
    let source_handle = source.as_ref().and_then(open);
    let target_handle = match (&source, &target) {
        // One device on both sides: one handle, as the worker opens it.
        (Some(one), Some(other)) if one == other => source_handle.clone(),
        (_, Some(other)) => open(other),
        _ => None,
    };
    JobSides {
        source: source_handle,
        target: target_handle,
    }
}

/// The handle a background task of the app uses instead of `handle` (a
/// mirror run, a storage analysis): a new handle of the origin `Job` when
/// `handle` is one a person browses with, else `handle` itself.
pub fn job_handle(handle: &BackendHandle) -> BackendHandle {
    user_target(&**handle)
        .and_then(|target| open_for_job(&target))
        .unwrap_or_else(|| handle.clone())
}

/// A new handle of the origin `Job` to `target`; `None` when it does not
/// open, and the job then runs over the handle it was given.
fn open_for_job(target: &PeerOpenTarget) -> Option<BackendHandle> {
    let origin = OperationOrigin::Job;
    let opened = match super::transfer_job_client::open_share_in_process(target, origin) {
        Some(opened) => opened,
        None => super::ipc_client::open_share_backend_for(target.clone(), origin),
    };
    opened.ok().map(|(_, handle, _)| handle)
}

#[cfg(test)]
#[path = "job_handles_tests.rs"]
mod tests;
