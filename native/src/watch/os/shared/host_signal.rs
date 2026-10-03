//! Change signals and change cursors reported by the embedding host (RV1,
//! contract V4). Android reports MediaStore changes, which its FUSE view of
//! shared storage does not show to inotify, and the MediaStore generation of
//! each volume, which lets the worker skip a verification run when nothing
//! changed. Paths are compared component-wise as given: pass canonical paths.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::paths::host_change_rel;
use super::service::{emit, roots};
use super::types::{Change, EventKind, WatchEvent};

static CURSORS: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());

fn cursors() -> MutexGuard<'static, Vec<(PathBuf, String)>> {
    // Every update leaves the list consistent; a panic elsewhere must not
    // take the cursors down with it.
    CURSORS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The host saw changes at or below each absolute path (a volume root means
/// anything on it). Every watch whose root contains such a path, or lies below
/// it, reports `Change { kind: Unknown }` for it, filtered like operating-system
/// events.
pub fn report_host_change(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    for (id, root) in roots() {
        let events: Vec<WatchEvent> = paths
            .iter()
            .filter_map(|path| host_change_rel(&root, path))
            .map(|rel| {
                WatchEvent::Change(Change {
                    rel,
                    kind: EventKind::Unknown,
                    is_dir: None,
                })
            })
            .collect();
        emit(id, events);
    }
}

/// Sets (`Some`) or removes (`None`) the host's change cursor of a storage
/// scope (Android: `"<MediaStore version>:<generation>"` of a volume root).
pub fn set_host_cursor(scope: &Path, cursor: Option<String>) {
    let mut cursors = cursors();
    cursors.retain(|(known, _)| known != scope);
    if let Some(cursor) = cursor {
        cursors.push((scope.to_path_buf(), cursor));
    }
}

/// The cursor of the longest reported scope that contains `path`; `None`
/// when the host reports none for it.
pub fn host_cursor(path: &Path) -> Option<String> {
    cursors()
        .iter()
        .filter(|(scope, _)| path.starts_with(scope))
        .max_by_key(|(scope, _)| scope.components().count())
        .map(|(_, cursor)| cursor.clone())
}
