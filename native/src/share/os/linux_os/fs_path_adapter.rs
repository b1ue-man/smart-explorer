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

/// Creates the file of a ranged upload's stage, exclusively (`create_new`).
pub(in crate::share) fn create_stage_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Physical identity of the directory `path` names (device and inode),
/// following links like `std::fs::metadata`; `None` when it cannot be read.
/// The host's policy cache compares it to skip a repeated ancestor walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DirectoryIdentity(u64, u64);

pub(super) fn directory_identity(path: &Path) -> Option<DirectoryIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    metadata
        .is_dir()
        .then(|| DirectoryIdentity(metadata.dev(), metadata.ino()))
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
