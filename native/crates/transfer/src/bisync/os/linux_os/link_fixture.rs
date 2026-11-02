use std::path::Path;

pub fn directory(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

pub fn remove_directory(link: &Path) { std::fs::remove_file(link).unwrap(); }
