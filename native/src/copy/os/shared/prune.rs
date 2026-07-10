use crate::types::FileEntry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) fn selected_directory_roots(entries: &[FileEntry]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = entries
        .iter()
        .filter(|entry| entry.is_dir && !entry.is_symlink)
        .filter_map(|entry| lexical_absolute(Path::new(entry.path.as_ref())).ok())
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

pub(super) fn prune_empty_dirs(roots: &[PathBuf], entries: &[FileEntry]) -> Vec<(String, String)> {
    let mut directories = HashSet::new();
    for entry in entries {
        let Ok(path) = lexical_absolute(Path::new(entry.path.as_ref())) else {
            continue;
        };
        let Some(root) = roots
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
        else {
            continue;
        };
        let mut current = if entry.is_dir {
            Some(path.as_path())
        } else {
            path.parent()
        };
        while let Some(directory) = current {
            if !directory.starts_with(root) {
                break;
            }
            directories.insert(directory.to_path_buf());
            if directory == root {
                break;
            }
            current = directory.parent();
        }
    }
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    let mut errors = Vec::new();
    for directory in directories {
        match std::fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) if errors.len() < 100 => {
                errors.push((directory.to_string_lossy().into_owned(), error.to_string()));
            }
            Err(_) => {}
        }
    }
    errors
}

fn lexical_absolute(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn pruning_never_crosses_an_explicit_directory_root() {
        let base = std::env::temp_dir().join(format!("se_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let selected = base.join("selected/child");
        let unrelated = base.join("unrelated/child");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir_all(&unrelated).unwrap();
        let entry = FileEntry {
            path: Arc::from(unrelated.join("gone").to_string_lossy().as_ref()),
            parent: Arc::from(unrelated.to_string_lossy().as_ref()),
            name: Arc::from("gone"),
            ext: Arc::from(""),
            size: 0,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir: false,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 0,
            id: None,
        };
        assert!(prune_empty_dirs(&[base.join("selected")], &[entry]).is_empty());
        assert!(unrelated.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
