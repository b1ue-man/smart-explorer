//! App trash for platforms without a system recycle bin for arbitrary files
//! (Android). Deleted items move by rename into
//! `<volume>/.SmartExplorer-Papierkorb/<id>/<name>` on the same storage volume,
//! next to an `<id>.json` record, and can be restored or purged later.
//!
//! Platform-neutral and inert until an embedding host calls `set_volumes`; the
//! desktop builds never do, so their recycle-bin behavior and scans stay as
//! they were.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[path = "core/record.rs"]
mod record;
#[path = "os/shared/store.rs"]
mod store;
#[cfg(test)]
#[path = "os/shared/store_tests.rs"]
mod store_tests;

pub use record::TrashEntry;

/// Folder name of the trash at every volume root.
pub const TRASH_DIR_NAME: &str = ".SmartExplorer-Papierkorb";

static VOLUMES: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Scans, indexes, transfers and sync walks skip this name as a protected
/// omission, but only while the app trash is active (volumes set).
pub fn excluded_name(name: &str) -> bool {
    is_excluded(name, || {
        !VOLUMES
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    })
}

/// The name check runs first, so the hot scan path takes no lock.
fn is_excluded(name: &str, trash_active: impl FnOnce() -> bool) -> bool {
    name == TRASH_DIR_NAME && trash_active()
}

/// Replaces the storage volumes (absolute roots) that carry an app trash.
pub fn set_volumes(volumes: Vec<PathBuf>) {
    *VOLUMES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = volumes;
}

/// Moves `path` into the trash of the volume that contains it. Never deletes:
/// if the move fails, the item stays where it was and the error is returned.
pub fn move_to_trash(path: &Path) -> io::Result<TrashEntry> {
    store::move_to_trash_in(&volumes(), path)
}

/// All entries of all volumes, newest first.
pub fn list() -> io::Result<Vec<TrashEntry>> {
    store::list_in(&volumes())
}

/// Moves an entry back to its original folder (recreated if needed) and
/// returns the restored path; an occupied name becomes `Name (2)` etc.
pub fn restore(id: &str) -> io::Result<PathBuf> {
    store::restore_in(&volumes(), id)
}

/// Deletes an entry permanently.
pub fn delete(id: &str) -> io::Result<()> {
    store::delete_in(&volumes(), id)
}

/// Deletes every entry older than `days` days and returns how many went.
pub fn purge_older_than(days: u32) -> io::Result<usize> {
    store::purge_older_than_in(&volumes(), days, store::now_ms())
}

fn volumes() -> Vec<PathBuf> {
    VOLUMES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
