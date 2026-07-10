use crate::types::{CopyOptions, FileEntry};
use std::path::{Path, PathBuf};

use super::super::platform;
use super::path_guard::validate_directory_target;

pub(super) fn rel_from_root(path_fwd: &str, root_fwd: &str) -> String {
    let relative = if root_fwd.is_empty() {
        Some(path_fwd.trim_start_matches('/'))
    } else if path_fwd == root_fwd {
        Some("")
    } else {
        path_fwd
            .strip_prefix(root_fwd)
            .and_then(|rest| rest.strip_prefix('/'))
    };
    if let Some(relative) = relative {
        if relative.is_empty() {
            return path_fwd.rsplit('/').next().unwrap_or(path_fwd).to_string();
        }
        relative.to_string()
    } else {
        path_fwd.rsplit('/').next().unwrap_or(path_fwd).to_string()
    }
}

pub(super) fn safe_rel_path(relative: &str) -> Option<PathBuf> {
    if Path::new(relative).components().any(|component| {
        matches!(
            component,
            std::path::Component::Prefix(_)
                | std::path::Component::RootDir
                | std::path::Component::ParentDir
        )
    }) {
        return None;
    }
    let mut output = PathBuf::new();
    for component in Path::new(relative).components() {
        match component {
            std::path::Component::Normal(part) => output.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    (!output.as_os_str().is_empty()).then_some(output)
}

pub(super) fn validate_seed_destinations(
    seeds: &[FileEntry],
    options: &CopyOptions,
) -> Result<(), String> {
    let root = platform::path_text(&options.root).map_err(|error| error.to_string())?;
    let root = root.trim_end_matches('/');
    for seed in seeds
        .iter()
        .filter(|entry| entry.is_dir && !entry.is_symlink)
    {
        let source = PathBuf::from(seed.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let target = if options.preserve_structure {
            let relative = rel_from_root(seed.path.as_ref(), root);
            let relative = safe_rel_path(&relative)
                .ok_or_else(|| format!("ungueltiger relativer Zielpfad fuer {}", seed.path))?;
            options.dest.join(relative)
        } else {
            options.dest.clone()
        };
        validate_directory_target(&source, &target).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_targets_reject_traversal() {
        assert!(safe_rel_path("sub/file.txt").is_some());
        assert!(safe_rel_path("../file.txt").is_none());
        assert!(safe_rel_path("sub/../../file.txt").is_none());
        assert!(safe_rel_path("/absolute.txt").is_none());
    }

    #[test]
    fn rel_from_root_respects_component_boundary() {
        assert_eq!(rel_from_root("C:/root/a.txt", "C:/root"), "a.txt");
        assert_eq!(rel_from_root("C:/rooted/a.txt", "C:/root"), "a.txt");
    }

    #[cfg(unix)]
    #[test]
    fn linux_backslash_is_a_filename_character() {
        assert_eq!(
            safe_rel_path("folder/a\\b.txt"),
            Some(PathBuf::from("folder/a\\b.txt"))
        );
    }
}
