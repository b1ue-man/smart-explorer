use std::path::{Component, Path, PathBuf};

pub(crate) fn validate_distinct_paths(paths: &[(&str, &Path)]) -> Result<(), String> {
    let mut keys = Vec::with_capacity(paths.len());
    for (label, path) in paths {
        if path.as_os_str().is_empty() {
            return Err(format!("{label}-Pfad ist leer"));
        }
        let key = comparison_key(path)?;
        for (previous_index, previous_key) in keys.iter().enumerate() {
            let (previous_label, previous_path) = paths[previous_index];
            if previous_key == &key || existing_paths_are_same(previous_path, path)? {
                return Err(format!(
                    "Updater-Pfade {previous_label} ({}) und {label} ({}) verweisen auf dieselbe Datei",
                    previous_path.display(),
                    path.display()
                ));
            }
        }
        keys.push(key);
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if !metadata.file_type().is_file() {
                return Err(format!(
                    "{label}-Pfad {} ist keine regulaere Datei",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn existing_paths_are_same(left: &Path, right: &Path) -> Result<bool, String> {
    let left_exists = std::fs::symlink_metadata(left).is_ok();
    let right_exists = std::fs::symlink_metadata(right).is_ok();
    if !left_exists || !right_exists {
        return Ok(false);
    }
    same_file::is_same_file(left, right).map_err(|error| {
        format!(
            "Dateiidentitaet {} / {} pruefen: {error}",
            left.display(),
            right.display()
        )
    })
}

fn comparison_key(path: &Path) -> Result<String, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("Arbeitsordner fuer {} lesen: {error}", path.display()))?
            .join(path)
    };
    let resolved = match std::fs::canonicalize(&absolute) {
        Ok(resolved) => resolved,
        Err(_) => {
            let parent = absolute
                .parent()
                .ok_or_else(|| format!("Pfad {} hat keinen Elternordner", path.display()))?;
            let resolved_parent =
                std::fs::canonicalize(parent).unwrap_or_else(|_| normalize_lexically(parent));
            match absolute.file_name() {
                Some(name) => resolved_parent.join(name),
                None => resolved_parent,
            }
        }
    };
    let key = normalize_lexically(&resolved)
        .to_string_lossy()
        .into_owned();
    #[cfg(windows)]
    return Ok(key.replace('/', "\\").to_lowercase());
    #[cfg(not(windows))]
    Ok(key)
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_exact_and_lexical_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"app").unwrap();
        let lexical = dir.path().join("child").join("..").join("target");

        assert!(validate_distinct_paths(&[("target", &target), ("staged", &target)]).is_err());
        assert!(validate_distinct_paths(&[("target", &target), ("staged", &lexical)]).is_err());
    }

    #[test]
    fn rejects_hardlink_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let staged = dir.path().join("staged");
        std::fs::write(&target, b"app").unwrap();
        std::fs::hard_link(&target, &staged).unwrap();

        assert!(validate_distinct_paths(&[("target", &target), ("staged", &staged)]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let staged = dir.path().join("staged");
        std::fs::write(&target, b"app").unwrap();
        std::os::unix::fs::symlink(&target, &staged).unwrap();

        assert!(validate_distinct_paths(&[("target", &target), ("staged", &staged)]).is_err());
    }
}
