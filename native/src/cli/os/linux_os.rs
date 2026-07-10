use std::io;
use std::os::unix::fs::MetadataExt;

pub(super) fn local_path(path: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

pub(super) fn same_file(left: &str, right: &str) -> io::Result<bool> {
    // Keep both files open so neither device/inode pair can be recycled between
    // the two metadata reads.
    let left = std::fs::File::open(left)?;
    let right = std::fs::File::open(right)?;
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok((left.dev(), left.ino()) == (right.dev(), right.ino()))
}
