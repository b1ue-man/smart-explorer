//! Local target folders: plain folders only, never a link or junction that
//! could redirect a copy elsewhere. Local copies keep refusing a link
//! anywhere on the target's path (as the copy module always did); downloads
//! resolve the chosen folder once, so a home or Downloads folder relocated
//! by a link or junction works, and check only what lies below it.
use super::super::super::platform::canonical_folder;
use super::super::super::walk_listers::native;
use std::io;
use std::path::{Component, Path, PathBuf};

/// The target folder a job writes below: for a local copy `target_dir`
/// itself (every folder on its path plain), for a download the folder it
/// resolves to.
pub(crate) fn local_target_root(source_is_local: bool, target_dir: &str) -> Result<String, String> {
    let root = if source_is_local {
        prepare_local_root(target_dir).map(|()| target_dir.to_string())
    } else {
        resolve_local_root(target_dir)
    };
    root.map_err(|error| format!("Zielordner „{target_dir}“: {error}"))
}

/// Creates one local folder or accepts an existing plain one; links,
/// junctions and files in its place are refused. True when created now.
pub(crate) fn ensure_plain_dir(path: &Path) -> io::Result<bool> {
    match crate::local_access::symlink_metadata(path) {
        Ok(metadata) => validate_plain_dir(path, &metadata).map(|()| false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match std::fs::create_dir(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                validate_plain_dir(path, &crate::local_access::symlink_metadata(path)?)
                    .map(|()| false)
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn validate_plain_dir(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if crate::local_access::metadata_is_link_like(path, metadata) {
        // Not a permission problem of the whole target: only what would go
        // through this link is refused.
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Zielordner ist ein Link oder Reparse-Punkt: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("Ziel ist kein Ordner: {}", path.display()),
        ));
    }
    Ok(())
}

/// The local target folder of a local copy and every ancestor: plain folders
/// (created when missing), never a link that could redirect the copy.
pub(crate) fn prepare_local_root(root: &str) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in absolute(root)?.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => return Err(parent_component()),
            Component::Normal(name) => {
                current.push(name);
                ensure_plain_dir(&current)?;
            }
        }
    }
    Ok(())
}

/// The chosen target folder of a download, resolved once: links and
/// junctions on its path, and the folder itself, lead where they point (a
/// relocated home or Downloads folder), and a later change of such a link
/// cannot redirect the job. Missing folders below the existing part are
/// created as plain folders. The resolved path, with `/` separators.
pub(crate) fn resolve_local_root(root: &str) -> io::Result<String> {
    let path = absolute(root)?;
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(parent_component());
    }
    let mut existing = path.as_path();
    let mut missing = Vec::new();
    while !existing.try_exists()? {
        let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "kein vorhandener übergeordneter Ordner",
            ));
        };
        missing.push(name.to_os_string());
        existing = parent;
    }
    let mut resolved = canonical_folder(existing)?;
    if !std::fs::metadata(&resolved)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("Ziel ist kein Ordner: {}", resolved.display()),
        ));
    }
    for name in missing.into_iter().rev() {
        resolved.push(name);
        ensure_plain_dir(&resolved)?;
    }
    Ok(resolved.to_string_lossy().replace('\\', "/"))
}

fn absolute(root: &str) -> io::Result<PathBuf> {
    let path = native(root);
    Ok(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    })
}

fn parent_component() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Zielordner enthält „..“")
}
