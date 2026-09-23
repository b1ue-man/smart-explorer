use std::path::Path;

pub(crate) fn directory(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

pub(crate) fn remove_directory(link: &Path) { std::fs::remove_file(link).unwrap(); }
