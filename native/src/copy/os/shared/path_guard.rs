use std::io;
use std::path::{Component, Path, PathBuf};

use super::super::platform;

pub(super) fn validate_directory_target(src: &Path, target: &Path) -> io::Result<()> {
    let source = comparable_path(src)?;
    let target = comparable_path(target)?;
    let prefix = if source.ends_with('/') {
        source.clone()
    } else {
        format!("{source}/")
    };
    if source == target || target.starts_with(&prefix) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot copy or move a directory into itself or its descendant: {} -> {}",
                src.display(),
                target
            ),
        ));
    }
    Ok(())
}

pub(super) fn prepare_target_parent(root: &Path, target: &Path) -> io::Result<()> {
    let root = absolute_normalized(root)?;
    let target = absolute_normalized(target)?;
    let relative = target.strip_prefix(&root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "destination escapes the selected root: {} is outside {}",
                target.display(),
                root.display()
            ),
        )
    })?;
    if relative.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file destination cannot equal the selected destination directory",
        ));
    }

    ensure_plain_directory_tree(&root)?;
    let resolved_root = std::fs::canonicalize(&root)?;
    let parent_relative = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut current = root.clone();
    for component in parent_relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination contains an unsafe path component",
            ));
        };
        current.push(name);
        ensure_plain_directory(&current)?;
        let resolved = std::fs::canonicalize(&current)?;
        if !resolved.starts_with(&resolved_root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "destination ancestor resolves outside the selected root: {}",
                    current.display()
                ),
            ));
        }
    }

    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if platform::metadata_is_link_like(&metadata) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "destination is a link or reparse point: {}",
                target.display()
            ),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn ensure_plain_directory_tree(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(std::path::MAIN_SEPARATOR_STR),
            Component::Normal(name) => {
                current.push(name);
                ensure_plain_directory(&current)?;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "destination root contains a parent component",
                ))
            }
        }
    }
    Ok(())
}

fn ensure_plain_directory(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_plain_directory(path, &metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match std::fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(path)?;
                validate_plain_directory(path, &metadata)
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn validate_plain_directory(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if platform::metadata_is_link_like(metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "destination ancestor is a link or reparse point: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!(
                "destination ancestor is not a directory: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn absolute_normalized(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "path escapes its filesystem root",
                    ));
                }
            }
            Component::Normal(name) => normalized.push(name),
        }
    }
    Ok(normalized)
}

fn comparable_path(path: &Path) -> io::Result<String> {
    let absolute = absolute_normalized(path)?;
    let mut probe = absolute;
    let mut missing = Vec::new();
    while !probe.try_exists()? {
        let name = probe.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "path has no existing ancestor")
        })?;
        missing.push(name.to_os_string());
        probe = probe
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "path has no parent"))?
            .to_path_buf();
    }
    let mut resolved = std::fs::canonicalize(probe)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    let normalized = resolved.to_string_lossy().replace('\\', "/");
    Ok(if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn selected_link_root_is_rejected_before_writing_outside() {
        let base = std::env::temp_dir().join(format!(
            "se-copy-root-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let victim = base.join("victim");
        let root = base.join("selected");
        std::fs::create_dir_all(&victim).unwrap();
        symlink(&victim, &root).unwrap();
        assert!(prepare_target_parent(&root, &root.join("file.txt")).is_err());
        assert!(!victim.join("file.txt").exists());
        std::fs::remove_file(root).ok();
        std::fs::remove_dir_all(base).ok();
    }
}
