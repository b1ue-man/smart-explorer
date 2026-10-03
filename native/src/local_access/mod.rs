//! Ordinary local reads with an explicitly consented, read-only Windows fallback.
use std::{ffi::OsString, fs::File, io, path::Path};

#[path = "core/entry_error.rs"]
mod entry_error;
#[cfg(windows)]
#[path = "os/windows/mod.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod platform;
#[path = "core/protocol.rs"]
mod protocol;
#[path = "core/regular.rs"]
mod regular;
pub(crate) use entry_error::EntryError;
pub(crate) use platform::{DirectoryHandle, QuarantinedChild};
pub(crate) use regular::{FinalLink, NotRegular};

#[cfg(test)]
#[path = "directory_handle_tests.rs"]
mod directory_handle_tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EntryKind {
    File,
    Directory,
    Link,
    #[default]
    Other,
}

#[derive(Default)]
pub(crate) struct LocalEntry {
    pub name: OsString,
    pub kind: EntryKind,
    pub is_dir: bool,
    pub is_link_like: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub hidden: bool,
    pub system: bool,
    pub unreachable: bool,
}

/// Link boundary and data-stream class of one entry's metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MetadataClass {
    /// Symlink, junction or name-surrogate reparse point: a walk boundary.
    pub link_like: bool,
    /// FIFO, socket or device (Windows: device attribute, AF_UNIX/LX tags).
    pub special: bool,
}

pub(crate) use platform::{
    can_request_access, display_path, metadata_class, metadata_is_link_like, normalize_scan_root,
    parallel_scan_allowed, read_directory, request_access, run_helper_if_requested,
};

pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    platform::open_read(path)
}

/// Open `path` for reading only if it is a regular file: it never waits on a
/// FIFO or reads a device, and refuses folders, special files and, with
/// `FinalLink::Refuse`, a link at the last component (`NotRegular` inside an
/// `InvalidInput` error). Windows data reparse points (cloud placeholders,
/// WOF, dedup) are regular files.
pub(crate) fn open_regular(path: &Path, final_link: FinalLink) -> io::Result<File> {
    platform::open_regular(path, final_link)
}

/// Classify the opened object itself, never a path that may have changed.
pub(crate) fn classify_open_file(file: &File) -> io::Result<MetadataClass> {
    platform::classify_open_file(file)
}

/// Harden only an object owned by the effective caller, before any private
/// bytes are read/written. Refuses redirects, special files and hardlinked
/// files. Windows migration requires READ_CONTROL and WRITE_DAC on the handle.
pub(crate) fn secure_private_handle(file: &File, is_directory: bool) -> io::Result<()> {
    platform::secure_private_handle(file, is_directory)
}

pub(crate) fn symlink_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    platform::symlink_metadata(path)
}

pub(crate) fn system_time_ms(time: std::time::SystemTime) -> i64 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as i64,
        Err(error) => -(error.duration().as_millis() as i64),
    }
}
