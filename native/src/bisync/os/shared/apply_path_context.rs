//! One path context per apply run: facts that are fixed for the run are
//! established once per side instead of once per check (G4 MS1/MS6).
//!
//! - The target limits of a sync root (filesystem naming rules, largest file;
//!   remote: one `statvfs`/capabilities request) belong to that root and do
//!   not change during a run.
//! - Whether a directory below a root is a mount point (and whether it is a
//!   remembered nested mount) is checked once per directory and run.
//! - A stage's parent folder is created (`mkdir_all`, on SFTP one request
//!   per component) once per folder and run; a stage that then fails with
//!   `NotFound` forgets it, so the retry creates it again.
//!
//! Every leaf check stays fresh: each guard still stats the root and every
//! path component, so links, junctions, specials and files replacing a
//! parent are found exactly as before. Without an installed context (scans,
//! quick mirror, restores) every call computes its fact directly, as before.
use super::omissions::OmissionKind;
use crate::vfs::{Backend, TargetLimits};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex};

/// Per-run memory of root and directory facts; see the module comment.
#[derive(Default)]
pub(crate) struct PathContext {
    /// (backend address, root) → limits. Backends outlive the run that
    /// installs this context, so their address identifies them within it.
    limits: Mutex<HashMap<(usize, String), TargetLimits>>,
    /// (backend address, root, directory, cross_mounts) → mount omission.
    mounts: Mutex<HashMap<(usize, String, String, bool), Option<OmissionKind>>>,
    /// (backend address, folder) this run created or found as a folder.
    parents: Mutex<HashSet<(usize, String)>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<PathContext>>> = const { RefCell::new(None) };
}

/// Restores the previous context of this thread, also on unwinding.
struct Restore(Option<Arc<PathContext>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        CURRENT.with(|current| *current.borrow_mut() = previous);
    }
}

impl PathContext {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

/// Runs `work` on this thread with `context` installed.
pub(crate) fn scoped<T>(context: &Arc<PathContext>, work: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|current| current.borrow_mut().replace(context.clone()));
    let _restore = Restore(previous);
    work()
}

fn current() -> Option<Arc<PathContext>> {
    CURRENT.with(|current| current.borrow().clone())
}

fn address(backend: &dyn Backend) -> usize {
    backend as *const dyn Backend as *const () as usize
}

/// The target limits of `root`, once per run and root.
pub(crate) fn target_limits(backend: &dyn Backend, root: &str) -> TargetLimits {
    let Some(context) = current() else {
        return crate::vfs::target_limits(backend, root);
    };
    let key = (address(backend), root.to_string());
    if let Some(limits) = context
        .limits
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
    {
        return *limits;
    }
    let limits = crate::vfs::target_limits(backend, root);
    context
        .limits
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, limits);
    limits
}

/// The mount omission of the local directory `path` below `root`, once per
/// run and directory. Errors are never remembered.
pub(crate) fn mount_omission(
    backend: &dyn Backend,
    root: &str,
    path: &str,
    cross_mounts: bool,
    compute: impl FnOnce() -> io::Result<Option<OmissionKind>>,
) -> io::Result<Option<OmissionKind>> {
    let Some(context) = current() else {
        return compute();
    };
    let key = (
        address(backend),
        root.to_string(),
        path.to_string(),
        cross_mounts,
    );
    if let Some(known) = context
        .mounts
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
    {
        return Ok(*known);
    }
    let fact = compute()?;
    context
        .mounts
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, fact);
    Ok(fact)
}

/// Creates the stage parent `folder` unless this run already did.
pub(crate) fn ensure_parent(backend: &dyn Backend, folder: &str) -> io::Result<()> {
    let Some(context) = current() else {
        return backend.mkdir_all(folder);
    };
    let key = (address(backend), folder.to_string());
    if context
        .parents
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&key)
    {
        return Ok(());
    }
    backend.mkdir_all(folder)?;
    context
        .parents
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key);
    Ok(())
}

/// The next stage below `folder` creates it again.
pub(crate) fn forget_parent(backend: &dyn Backend, folder: &str) {
    if let Some(context) = current() {
        context
            .parents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&(address(backend), folder.to_string()));
    }
}

#[cfg(test)]
#[path = "apply_path_context_tests.rs"]
mod tests;
