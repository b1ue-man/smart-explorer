use super::temp::{read_session_pid, session_tag, temp_root, PRESERVE_MARKER};
use std::path::{Path, PathBuf};

const MAX_RETAINED_SESSIONS: usize = 10_000;

struct RecoveryInventory {
    dirs: Vec<PathBuf>,
    total: usize,
}

pub(in crate::app) struct RecoveryDeletePlan {
    pub(in crate::app) discovered: usize,
    pub(in crate::app) directories: Vec<PathBuf>,
}

pub(in crate::app) fn recovery_session_count() -> std::io::Result<usize> {
    Ok(recovery_inventory()?.total)
}

pub(in crate::app) fn recovery_delete_plan() -> std::io::Result<RecoveryDeletePlan> {
    let inventory = recovery_inventory()?;
    Ok(RecoveryDeletePlan {
        discovered: inventory.total,
        directories: inventory.dirs,
    })
}

pub(in crate::app) fn remove_recovery_session_controlled<F>(
    directory: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: F,
) -> Result<crate::vfs::RecursiveDeleteReport, crate::vfs::RecursiveDeleteFailure>
where
    F: FnMut(crate::vfs::RecursiveDeleteProgress),
{
    if !is_direct_child(&temp_root(), directory) || !is_recovery_directory(directory) {
        return Err(crate::vfs::RecursiveDeleteFailure {
            error: std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "unsafe or changed recovery directory: {}",
                    directory.display()
                ),
            ),
            planned: 0,
            removed: 0,
        });
    }
    super::temp_delete::remove_owned_tree_controlled(&temp_root(), directory, cancel, progress)
}

fn recovery_inventory() -> std::io::Result<RecoveryInventory> {
    let root = temp_root();
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveryInventory {
                dirs: Vec::new(),
                total: 0,
            });
        }
        Err(error) => return Err(error),
    };
    let current = session_tag();
    let mut inventory = RecoveryInventory {
        dirs: Vec::new(),
        total: 0,
    };
    for entry in entries {
        let entry = entry?;
        let directory = entry.path();
        if entry.file_name().to_str() == Some(current)
            || !is_direct_child(&root, &directory)
            || !is_recovery_directory(&directory)
            || owner_is_live_or_unknown(&directory)
        {
            continue;
        }
        inventory.total = inventory.total.saturating_add(1);
        if inventory.dirs.len() < MAX_RETAINED_SESSIONS {
            inventory.dirs.push(directory);
        }
    }
    inventory.dirs.sort();
    Ok(inventory)
}

fn owner_is_live_or_unknown(directory: &Path) -> bool {
    read_session_pid(directory)
        .map(crate::app::platform_helpers::process_running)
        .unwrap_or(true)
}

fn is_recovery_directory(directory: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(directory) else {
        return false;
    };
    if !metadata.is_dir() || crate::app::upload_is_link_like(&metadata) {
        return false;
    }
    let Ok(marker) = std::fs::symlink_metadata(directory.join(PRESERVE_MARKER)) else {
        return false;
    };
    marker.is_file() && !crate::app::upload_is_link_like(&marker)
}

fn is_direct_child(root: &Path, candidate: &Path) -> bool {
    candidate.parent() == Some(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_direct_children_are_accepted() {
        let root = Path::new("/tmp/smart-explorer-recovery");
        assert!(is_direct_child(root, &root.join("session")));
        assert!(!is_direct_child(root, &root.join("session/child")));
        assert!(!is_direct_child(root, Path::new("/tmp/elsewhere")));
    }

    #[test]
    fn missing_owner_marker_fails_closed() {
        assert!(owner_is_live_or_unknown(Path::new(
            "/definitely/missing/smart-explorer-session"
        )));
    }
}
