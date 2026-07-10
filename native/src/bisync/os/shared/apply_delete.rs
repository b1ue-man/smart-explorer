use crate::vfs::Backend;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::apply_guard::{capture, revalidate, ExpectedFile};
use super::apply_retry::AttemptError;
use super::apply_transfer::{back_up_captured, verify_expected_content};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeleteGuardedPhase {
    BackingUp,
    Deleting,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn delete_guarded(
    backend: &dyn Backend,
    path: &str,
    rel: &str,
    expected: ExpectedFile,
    reversible: bool,
    versions_dir: &Path,
    use_recycle: bool,
    cancel: &AtomicBool,
) -> Result<(), AttemptError> {
    delete_guarded_with_progress(
        backend,
        path,
        rel,
        expected,
        reversible,
        versions_dir,
        use_recycle,
        cancel,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn delete_guarded_with_progress(
    backend: &dyn Backend,
    path: &str,
    rel: &str,
    expected: ExpectedFile,
    reversible: bool,
    versions_dir: &Path,
    use_recycle: bool,
    cancel: &AtomicBool,
    progress: impl FnMut(DeleteGuardedPhase),
) -> Result<(), AttemptError> {
    delete_guarded_with_progress_and_guard(
        backend,
        path,
        rel,
        expected,
        reversible,
        versions_dir,
        use_recycle,
        cancel,
        progress,
        || Ok(()),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn delete_guarded_with_progress_and_guard(
    backend: &dyn Backend,
    path: &str,
    rel: &str,
    expected: ExpectedFile,
    reversible: bool,
    versions_dir: &Path,
    use_recycle: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(DeleteGuardedPhase),
    mut pre_commit_guard: impl FnMut() -> io::Result<()>,
) -> Result<(), AttemptError> {
    let state =
        capture(backend, path, expected, "delete target").map_err(AttemptError::pre_commit)?;
    state
        .regular("delete target")
        .map_err(AttemptError::pre_commit)?;
    if reversible {
        progress(DeleteGuardedPhase::BackingUp);
        back_up_captured(
            backend,
            path,
            rel,
            versions_dir,
            &state,
            expected,
            Some(cancel),
        )
        .map_err(AttemptError::pre_commit)?;
    } else {
        verify_expected_content(backend, path, &state, expected, cancel)
            .map_err(AttemptError::pre_commit)?;
    }
    pre_commit_guard().map_err(AttemptError::pre_commit)?;
    revalidate(backend, path, &state, "delete target").map_err(AttemptError::pre_commit)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AttemptError::pre_commit(interrupted()));
    }
    progress(DeleteGuardedPhase::Deleting);
    let id = state
        .regular("delete target")
        .map_err(AttemptError::pre_commit)?
        .id
        .as_deref();
    let result = if use_recycle && backend.is_local() {
        trash::delete(path).map_err(|error| io::Error::other(format!("recycle failed: {error}")))
    } else {
        backend.remove_file_id(path, id)
    };
    result.map_err(AttemptError::commit_attempted)
}

fn interrupted() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "synchronization delete canceled",
    )
}
