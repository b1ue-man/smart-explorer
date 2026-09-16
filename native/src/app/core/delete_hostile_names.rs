//! Recycle-bin deletion for local names Win32 cannot address through the
//! shell. `trash` hands the path to `IFileOperation`, which parses it exactly
//! like Explorer does, so `NUL`, `report.` or `a<b` cannot be recycled under
//! their stored names. Such an entry is first renamed to a safe unique sibling
//! through the local VFS (whose verbatim source path reaches the stored entry),
//! then recycled under the new name. Permanent deletion needs no detour: the
//! local VFS addresses the entry verbatim.
use crate::types::{win32_name_issue, win32_safe_name};
use crate::vfs::{Backend, DeleteTarget, LocalBackend};
use std::path::PathBuf;

const MAX_RENAME_ATTEMPTS: u32 = 1000;

/// How a local target reached the Recycle Bin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum RecycleRoute {
    /// Recycled under its own name.
    Direct,
    /// Renamed to a Win32-safe sibling name first because the shell cannot
    /// parse the stored name; the Recycle Bin holds it under the new name.
    RenamedFirst,
}

/// Move `target` to the Recycle Bin. `win32_rules` says whether the local
/// filesystem is addressed through Win32 (then hostile names take the rename
/// detour); elsewhere every name is recycled directly.
pub(in crate::app) fn recycle_local_target(
    target: &DeleteTarget,
    win32_rules: bool,
) -> Result<RecycleRoute, String> {
    let name = target
        .path
        .rsplit('/')
        .next()
        .unwrap_or(target.path.as_str());
    let issue = if win32_rules {
        win32_name_issue(name)
    } else {
        None
    };
    let Some(issue) = issue else {
        return trash::delete(native_path(&target.path))
            .map(|()| RecycleRoute::Direct)
            .map_err(|error| error.to_string());
    };
    let backend = LocalBackend::new("/");
    let renamed = rename_to_safe_sibling(&backend, &target.path).map_err(|error| {
        format!(
            "„{name}“ ({}): Umbenennen vor dem Papierkorb fehlgeschlagen: {error}. \
             Alternative: endgültig löschen (Shift+Entf).",
            issue.label_de()
        )
    })?;
    let renamed_name = renamed
        .rsplit('/')
        .next()
        .unwrap_or(renamed.as_str())
        .to_string();
    trash::delete(native_path(&renamed))
        .map(|()| RecycleRoute::RenamedFirst)
        .map_err(|error| format!("{error}; der Eintrag heißt jetzt „{renamed_name}“"))
}

/// Rename `path` (forward slashes) to the first free Win32-safe sibling name
/// and return the new forward-slash path. Never replaces an existing entry.
pub(in crate::app) fn rename_to_safe_sibling(
    backend: &dyn Backend,
    path: &str,
) -> Result<String, String> {
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    let safe = win32_safe_name(name);
    for attempt in 0..MAX_RENAME_ATTEMPTS {
        let candidate = if attempt == 0 {
            safe.clone()
        } else {
            numbered(&safe, attempt + 1)
        };
        let candidate_path = if parent.is_empty() {
            candidate
        } else {
            format!("{parent}/{candidate}")
        };
        match backend.rename_no_replace(path, &candidate_path) {
            Ok(()) => return Ok(candidate_path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err(format!(
        "kein freier Name nach {MAX_RENAME_ATTEMPTS} Versuchen"
    ))
}

/// `report (2).txt` for `report.txt`; `_nul (2)` for `_nul`.
fn numbered(name: &str, index: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({index}).{extension}"),
        _ => format!("{name} ({index})"),
    }
}

fn native_path(forward: &str) -> PathBuf {
    PathBuf::from(forward.replace('/', std::path::MAIN_SEPARATOR_STR))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("se_hostile_rename_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn recursive_filter_task_numbered_names_keep_their_extension() {
        assert_eq!(numbered("report.txt", 2), "report (2).txt");
        assert_eq!(numbered("_nul", 3), "_nul (3)");
        assert_eq!(numbered(".hidden", 2), ".hidden (2)");
    }

    #[test]
    fn recursive_filter_task_safe_sibling_rename_never_replaces_an_existing_entry() {
        let root = temp_root();
        // Written through the local VFS so the stored name is exact on every OS.
        let backend = LocalBackend::new("/");
        let root_fwd = root.to_string_lossy().replace('\\', "/");
        let hostile = format!("{root_fwd}/nul.txt");
        let taken = format!("{root_fwd}/_nul.txt");
        for path in [&hostile, &taken] {
            let mut writer = backend.open_write_new(path).unwrap();
            std::io::Write::write_all(&mut writer, b"x").unwrap();
        }

        let renamed = rename_to_safe_sibling(&backend, &hostile).unwrap();
        assert_eq!(renamed, format!("{root_fwd}/_nul (2).txt"));
        assert!(backend.stat(&renamed).is_ok());
        assert!(backend.stat(&hostile).is_err());
        assert_eq!(
            backend.stat(&taken).unwrap().size,
            1,
            "the taken name is untouched"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recursive_filter_task_plain_names_never_take_the_rename_detour() {
        let target = DeleteTarget {
            path: "/definitely/missing/plain.txt".into(),
            id: None,
            is_dir: false,
            is_symlink: false,
        };
        // A missing plain file fails in `trash` itself, never in a rename step.
        let error = recycle_local_target(&target, true).unwrap_err();
        assert!(!error.contains("Umbenennen"), "{error}");
    }
}
