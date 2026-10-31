//! The folder the standalone agent keeps its digest cache in (the shared
//! protocol code asks `crate::support_dirs::app_data_dir`, which in the app is
//! the app data folder): `$XDG_CACHE_HOME/se-agent` or `~/.cache/se-agent`
//! (`%LOCALAPPDATA%\se-agent` on Windows), empty when the process has no home
//! (then the digests stay in memory). A `--serve-root` agent is confined to
//! its root (Landlock): its cache folder is outside it unless
//! `cache_is_inside` says otherwise, and `main.rs` then keeps the digests in
//! memory instead of failing to write a file at every flush.
use std::path::{Path, PathBuf};

pub(crate) fn app_data_dir() -> PathBuf {
    cache_base()
        .map(|base| base.join("se-agent"))
        .unwrap_or_default()
}

/// Whether the cache folder (`app_data_dir`) lies inside `root`, where a
/// confined agent may write. True when the process has no cache folder (there
/// is nothing to write). Both paths are resolved as far as they exist; the
/// part of the folder that does not exist yet is taken as written.
pub(crate) fn cache_is_inside(root: &Path) -> bool {
    let folder = app_data_dir();
    folder.as_os_str().is_empty() || resolved(&folder).starts_with(resolved(root))
}

/// `path` with every existing ancestor resolved (links, `.` and `..`).
fn resolved(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut base = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(base) {
            return missing
                .iter()
                .rev()
                .fold(real, |path, name| path.join(name));
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                base = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(not(windows))]
fn cache_base() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))
}

#[cfg(windows)]
fn cache_base() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| Path::new(&home).join(".cache")))
}
