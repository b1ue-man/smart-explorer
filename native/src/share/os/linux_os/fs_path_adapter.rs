//! Unix encoding and the existing Share path-containment rules.
use std::path::{Path, PathBuf};

pub(in crate::share) fn to_os_path(path: &str) -> PathBuf {
    let b = path.as_bytes();
    if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        PathBuf::from(format!("{path}/"))
    } else {
        PathBuf::from(path)
    }
}

pub(super) fn from_os_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn policy_contains(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

pub(super) fn canonical_contains(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
pub(super) fn create_directory_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
