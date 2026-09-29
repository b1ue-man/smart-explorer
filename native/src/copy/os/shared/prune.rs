//! After a move, the source folders that became empty are removed, deepest
//! first, never outside the selected folders and never a folder that still
//! holds anything (a file that could not be moved keeps its folders).
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Reported cleanup problems per move (the rest is counted by the caller's
/// issue log anyway).
const MAX_PRUNE_ERRORS: usize = 100;

/// Removes the empty ones among `directories` that lie inside one of the
/// selected folders `roots`.
pub(crate) fn prune_empty_dirs(
    roots: &[PathBuf],
    directories: &[PathBuf],
) -> Vec<(String, String)> {
    let roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|root| lexical_absolute(root).ok())
        .collect();
    let mut inside: Vec<PathBuf> = directories
        .iter()
        .filter_map(|directory| lexical_absolute(directory).ok())
        .filter(|directory| roots.iter().any(|root| directory.starts_with(root)))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    inside.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    let mut errors = Vec::new();
    for directory in inside {
        match std::fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) if errors.len() < MAX_PRUNE_ERRORS => {
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

    #[test]
    fn pruning_never_crosses_an_explicit_directory_root() {
        let base = std::env::temp_dir().join(format!("se_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let selected = base.join("selected/child");
        let unrelated = base.join("unrelated/child");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir_all(&unrelated).unwrap();
        let errors = prune_empty_dirs(
            &[base.join("selected")],
            &[unrelated.clone(), base.join("unrelated"), selected.clone()],
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(unrelated.exists());
        assert!(
            !selected.exists(),
            "an empty folder inside the selection goes"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
