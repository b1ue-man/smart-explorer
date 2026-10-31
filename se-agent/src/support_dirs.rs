//! The folder the standalone agent keeps its digest cache in (the shared
//! protocol code asks `crate::support_dirs::app_data_dir`, which in the app is
//! the app data folder): `$XDG_CACHE_HOME/se-agent` or `~/.cache/se-agent`
//! (`%LOCALAPPDATA%\se-agent` on Windows), empty when the process has no home
//! (then the digests stay in memory).
use std::path::{Path, PathBuf};

pub(crate) fn app_data_dir() -> PathBuf {
    cache_base()
        .map(|base| base.join("se-agent"))
        .unwrap_or_default()
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
