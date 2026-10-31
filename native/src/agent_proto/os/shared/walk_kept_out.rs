//! Folders a walk of the agent lists but never enters: Linux pseudo
//! filesystems (`fs::is_pseudo_dir`, kernel and device state, no files) and
//! the folder that holds the process's digest cache (`hash_cache`). The walk
//! sends the folder's row and an omission of the same path
//! (`omission::UNREADABLE`, the wire's reason for a path the walk does not
//! read): a sync side treats an omitted path as protected, never as a
//! deletion, and keeps its counterpart on the other side and its entry in the
//! baseline. The cache file is rewritten or extended at the end of each walk,
//! so it would otherwise change under every run and be transferred each time.
use std::path::Path;

/// Why a walk does not enter the folder `path` (the text of the omission), or
/// `None` when it enters it.
pub(super) fn kept_out_dir(path: &Path) -> Option<&'static str> {
    if path.to_str().is_some_and(super::fs::is_pseudo_dir) {
        Some("Pseudo-Dateisystem: Inhalt wird nicht durchlaufen")
    } else if super::hash_cache::is_cache_folder(path) {
        Some("Ordner des Prüfsummen-Caches: Inhalt wird nicht durchlaufen")
    } else {
        None
    }
}
