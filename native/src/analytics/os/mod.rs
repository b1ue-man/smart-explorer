use std::{ffi::OsString, io, path::Path};

#[cfg(windows)]
mod windows;
#[cfg(not(windows))]
#[path = "shared/local_directory.rs"]
mod platform;
#[cfg(windows)]
use windows as platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EntryKind {
    File,
    Directory,
    Link,
    Other,
}

pub(super) struct LocalEntry {
    pub name: OsString,
    pub kind: EntryKind,
    pub size: u64,
}

pub(super) fn read_directory(
    path: &Path,
) -> io::Result<impl Iterator<Item = io::Result<LocalEntry>>> {
    platform::read_directory(path)
}
