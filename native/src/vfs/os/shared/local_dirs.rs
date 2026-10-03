//! Folder creation of the local backend. Links and junctions at or above the
//! backend's root belong to the chosen location (relocated home folders,
//! `/sdcard`, volumes mounted in folders); below it every existing component
//! must be a plain folder, so a link inside the tree never redirects writes
//! out of it. An earlier successful check is not proof that a path still
//! names that folder: every creation checks the current components.
use std::io;
use std::path::{Component, Path, PathBuf};

use super::local_platform;

pub(super) struct FolderGuard {
    root: PathBuf,
}

impl FolderGuard {
    pub(super) fn new(root: &str) -> Self {
        let root = local_platform::to_os(root);
        Self { root }
    }

    /// Create `path` and its missing parents: the root as given (links on its
    /// way are followed), every folder below it as a plain folder. A path
    /// outside the root is checked from the filesystem root.
    pub(super) fn mkdir_all(&self, path: &Path) -> io::Result<()> {
        let target = absolute(path)?;
        let root = absolute(&self.root)?;
        let Ok(below) = target.strip_prefix(&root) else {
            return mkdir_all_plain(&target);
        };
        ensure_root(&root)?;
        let mut current = root.clone();
        for component in below.components() {
            match component {
                Component::Normal(name) => {
                    current.push(name);
                    ensure_plain_component(&current)?;
                }
                Component::CurDir => {}
                _ => return Err(parent_component()),
            }
        }
        Ok(())
    }
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn parent_component() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "directory creation contains a parent component",
    )
}

/// The chosen root and its ancestors are taken as they are, links included.
fn ensure_root(root: &Path) -> io::Result<()> {
    match std::fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("root is not a directory: {}", root.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            local_platform::check_new_name(root)?;
            std::fs::create_dir_all(root)
        }
        Err(error) => Err(error),
    }
}

/// Create each missing component while refusing existing symlinks, junctions,
/// and other reparse points. `std::fs::create_dir_all` follows such ancestors,
/// which can redirect a selected sync/copy root outside its authorized tree.
pub(super) fn mkdir_all_plain(path: &Path) -> io::Result<()> {
    let absolute = absolute(path)?;
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => return Err(parent_component()),
            Component::Normal(name) => {
                current.push(name);
                ensure_plain_component(&current)?;
            }
        }
    }
    Ok(())
}

/// One folder whose parent exists: an existing plain folder is fine, a
/// missing one is created (names Windows cannot hold are refused).
pub(super) fn ensure_plain_component(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_plain_component(path, &metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            local_platform::check_new_name(path)?;
            match std::fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    validate_plain_component(path, &std::fs::symlink_metadata(path)?)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn validate_plain_component(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if crate::local_access::metadata_is_link_like(path, metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "directory ancestor is a link or reparse point: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("directory ancestor is not a directory: {}", path.display()),
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod mkdir_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn mkdir_all_rejects_link_ancestor() {
        let base = std::env::temp_dir().join(format!(
            "se-vfs-mkdir-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let victim = base.join("victim");
        let link = base.join("link");
        std::fs::create_dir_all(&victim).unwrap();
        symlink(&victim, &link).unwrap();
        assert!(mkdir_all_plain(&link.join("child")).is_err());
        assert!(!victim.join("child").exists());
        std::fs::remove_file(link).ok();
        std::fs::remove_dir_all(base).ok();
    }
}
