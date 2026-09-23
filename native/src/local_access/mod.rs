//! Ordinary local reads with an explicitly consented, read-only Windows fallback.
use std::{ffi::OsString, fs::File, io, path::Path};

#[cfg(windows)]
#[path = "os/windows/mod.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod platform;
#[path = "core/protocol.rs"]
mod protocol;

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

pub(crate) use platform::{
    can_request_access, display_path, metadata_is_link_like, normalize_scan_root,
    parallel_scan_allowed, read_directory, request_access, run_helper_if_requested,
};

pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    platform::open_read(path)
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
