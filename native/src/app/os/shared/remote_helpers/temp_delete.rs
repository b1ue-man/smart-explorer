use std::io;
use std::path::{Component, Path};

/// Delete an app-owned temp subtree through the bounded VFS delete planner.
/// The lexical containment check rejects `..` and the planner treats every
/// symlink/junction/reparse point as a non-recursive leaf.
pub(super) fn remove_owned_tree(authorized_root: &Path, target: &Path) -> io::Result<()> {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    match remove_owned_tree_controlled(authorized_root, target, &cancel, |_| {}) {
        Ok(report) if report.status == crate::vfs::RecursiveDeleteStatus::Complete => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "temp cleanup canceled",
        )),
        Err(failure) => Err(failure.error),
    }
}

pub(super) fn remove_owned_tree_controlled<F>(
    authorized_root: &Path,
    target: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: F,
) -> Result<crate::vfs::RecursiveDeleteReport, crate::vfs::RecursiveDeleteFailure>
where
    F: FnMut(crate::vfs::RecursiveDeleteProgress),
{
    validate_owned_target(authorized_root, target).map_err(failure_before_plan)?;
    let backend = crate::vfs::LocalBackend::new("/");
    let path = target.to_string_lossy().replace('\\', "/");
    let metadata = crate::vfs::Backend::stat(&backend, &path).map_err(failure_before_plan)?;
    crate::vfs::remove_entry_controlled(
        &backend,
        &crate::vfs::DeleteTarget {
            path,
            id: metadata.id,
            is_dir: metadata.is_dir,
            is_symlink: metadata.is_symlink,
        },
        cancel,
        progress,
    )
}

fn failure_before_plan(error: io::Error) -> crate::vfs::RecursiveDeleteFailure {
    crate::vfs::RecursiveDeleteFailure {
        error,
        planned: 0,
        removed: 0,
    }
}

fn validate_owned_target(authorized_root: &Path, target: &Path) -> io::Result<()> {
    let relative = target.strip_prefix(authorized_root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("temp cleanup escaped its root: {}", target.display()),
        )
    })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("invalid temp cleanup target: {}", target.display()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_target_must_be_a_plain_descendant() {
        let root = Path::new("/tmp/se-owned");
        assert!(validate_owned_target(root, &root.join("session/item")).is_ok());
        assert!(validate_owned_target(root, root).is_err());
        assert!(validate_owned_target(root, &root.join("../victim")).is_err());
        assert!(validate_owned_target(root, Path::new("/tmp/victim")).is_err());
    }
}
