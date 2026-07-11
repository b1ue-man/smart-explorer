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

pub(super) fn validate_connection_protocol(protocol: crate::creds::Protocol) -> Result<(), String> {
    if protocol == crate::creds::Protocol::Share {
        return Err(
            "UNC authentication is only supported on Windows; mount the share with CIFS and use its local mount path"
                .to_string(),
        );
    }
    Ok(())
}
