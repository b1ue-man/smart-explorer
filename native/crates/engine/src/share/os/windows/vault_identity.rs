//! Windows identity check of the vault drives behind `in_vault_drive` (E35,
//! SEC-FS-WINDOWS-PFAD). A spelling says nothing about the object it names:
//! case, 8.3 short name, junction and `\\?\` prefix can name the same folder.
//! The check holds the nearest existing folder of a path open and compares
//! its volume serial number and file ID with the drive roots.
use std::path::{Path, PathBuf};

use crate::local_access::DirectoryHandle;

/// Whether `path` lies in one of the vault drive `roots` by object identity.
///
/// Roots that cannot be reached now are no candidates, as in
/// `ensure_handle_allowed`. When the identity of an existing folder of `path`
/// cannot be established, the path counts as inside: a place that cannot be
/// checked is never shared. A path without any existing folder names no
/// object and so no vault object (an offline share, a missing drive letter);
/// it keeps the answer of the spelling check, and every later access is
/// checked again on its handle.
pub(super) fn in_vault_identity(path: &Path, roots: &[PathBuf]) -> bool {
    let reachable: Vec<PathBuf> = roots
        .iter()
        .filter(|root| std::fs::metadata(root).is_ok())
        .cloned()
        .collect();
    if reachable.is_empty() {
        return false;
    }
    let Some(folder) = nearest_folder(path) else {
        return false;
    };
    DirectoryHandle::open_root(&folder)
        .and_then(|handle| handle.is_within_any(&reachable))
        .unwrap_or(true)
}

/// `path` itself when it is a folder, otherwise its nearest existing ancestor.
fn nearest_folder(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|candidate| candidate.is_dir())
        .map(Path::to_path_buf)
}
