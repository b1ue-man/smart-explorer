//! Windows encoding and the existing Share path-containment rules.
use std::path::{Path, PathBuf};

use super::fs_policy::{normalized, within};

pub(in crate::share) fn to_os_path(path: &str) -> PathBuf {
    let b = path.as_bytes();
    let rooted;
    let path = if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        rooted = format!("{path}/");
        rooted.as_str()
    } else {
        path
    };
    PathBuf::from(path.replace('/', "\\"))
}

pub(super) fn from_os_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn policy_contains(root: &Path, path: &Path) -> bool {
    within(
        &normalized(&path.to_string_lossy()),
        &normalized(&root.to_string_lossy()),
    )
}

pub(super) fn canonical_contains(root: &Path, path: &Path) -> bool {
    // Canonical target resolution retains its original component comparison.
    path.starts_with(root)
}

#[cfg(test)]
pub(super) fn create_directory_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
