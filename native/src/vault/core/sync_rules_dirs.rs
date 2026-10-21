//! Folder actions of vault sync plans (vault plan C5.4, R12): a node folder
//! (`d/XX/YYYY…/<name>.c9r` or `.c9s`) exists only while it holds its type
//! file (`dir.c9r`, `symlink.c9r`) or the long-name pair (`contents.c9r`,
//! `name.c9s`). An empty one is a node of unknown type, which CryptoFS reports
//! as `UnknownType` (critical).
//!
//! The general engine plans folders on its own: it creates a folder only one
//! side has on the other side, and removes it there only when its history
//! knows the folder on both sides. A folder that the first run copied
//! implicitly (as the parent of a copied file) is not in that history, so a
//! node folder whose files the other side deleted would be created empty on
//! the deleting side and stay empty on the side that deleted its files.
//! Pure decisions on relative cipher paths: no I/O and no key.
use crate::bisync::{DirAction, PairSide};
use crate::cryptomator::{CIPHER_SUFFIX, DATA_DIR, SHORTENED_SUFFIX};

/// Components of `rel` below `d/` (`d`, bucket, content folder, …).
fn components(rel: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = rel.trim_start_matches('/').split('/').collect();
    (parts.first() == Some(&DATA_DIR)).then_some(parts)
}

fn is_holder_name(name: &str) -> bool {
    name.ends_with(CIPHER_SUFFIX) || name.ends_with(SHORTENED_SUFFIX)
}

/// Whether `rel` is a node folder or lies inside one (`d/XX/YYYY…/<holder>`).
/// Files at the depth of a node folder (`dirid.c9r`, short file names) are
/// no folders, so a folder path of that shape always is a node folder.
fn in_node_folder(rel: &str) -> bool {
    components(rel).is_some_and(|parts| parts.len() >= 4 && is_holder_name(parts[3]))
}

/// The node folder holding the file `rel` (`d/XX/YYYY…/<holder>/<file>`).
fn holder_of_file(rel: &str) -> Option<String> {
    let parts = components(rel)?;
    (parts.len() == 5 && is_holder_name(parts[3])).then(|| parts[..4].join("/"))
}

/// R12: no node folder is created empty, and a node folder whose files a
/// run deletes on `side` is removed there (the engine removes it only while
/// it is empty; a failed or held deletion leaves it, and the next run plans
/// it again), unless the run also writes into it on that side (a link
/// replaced by a folder of the same name keeps its node folder).
/// `deletions` are the planned deletions that are not left out of this run,
/// `writes` the planned copies with the side they write to. Removals stay in
/// front of the removal of their parents.
pub(super) fn node_folders(
    dirs: &mut Vec<DirAction>,
    deletions: &[(PairSide, String)],
    writes: &[(PairSide, String)],
) {
    // A node folder gets its files by copy, and a copy creates the folder.
    dirs.retain(|dir| !matches!(dir, DirAction::Create { .. }) || !in_node_folder(dir.rel()));
    let mut removals: Vec<DirAction> = Vec::new();
    for (side, rel) in deletions {
        let Some(holder) = holder_of_file(rel) else {
            continue;
        };
        let inside = format!("{holder}/");
        if writes
            .iter()
            .any(|(at, written)| at == side && written.starts_with(&inside))
        {
            continue;
        }
        let removal = DirAction::Remove {
            side: *side,
            rel: holder,
        };
        if !dirs.contains(&removal) && !removals.contains(&removal) {
            removals.push(removal);
        }
    }
    let at = dirs
        .iter()
        .position(|dir| matches!(dir, DirAction::Remove { .. }))
        .unwrap_or(dirs.len());
    dirs.splice(at..at, removals);
}
