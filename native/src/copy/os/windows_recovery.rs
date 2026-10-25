//! Conditional recovery on the same expected Windows DELETE handle.
use std::{ffi::OsStr, fs::File, io};
use crate::local_access::ConfinedDirectory;

pub(crate) fn recover_transfer_child(
    parent: &ConfinedDirectory,
    name: &OsStr,
    expected_identity: &str,
    destination: Option<&OsStr>,
) -> io::Result<()> {
    let expected = parent.entry(name)?;
    if crate::copy::local_file_identity_handle(&expected)? != expected_identity {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "transfer recovery file identity changed"));
    }
    parent.recover_expected_file(name, &expected, destination)
}

pub(crate) fn recover_transfer_child_live(
    parent: &ConfinedDirectory, name: &OsStr, expected: &File,
    destination: Option<&OsStr>,
) -> io::Result<()> {
    parent.recover_expected_file(name, expected, destination)
}
