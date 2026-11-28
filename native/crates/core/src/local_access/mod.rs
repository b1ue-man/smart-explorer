//! Ordinary local reads with an explicitly consented, read-only Windows fallback.
use std::{ffi::OsString, fs::File, io, path::Path};

#[path = "core/entry_error.rs"]
mod entry_error;
/// NTFS MFT parsing (pure); the Windows volume reader is `platform::mft`.
#[path = "core/mft/mod.rs"]
pub mod mft;
#[cfg(windows)]
#[doc(hidden)]
#[path = "os/windows/mod.rs"]
pub mod platform;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod platform;
#[path = "core/protocol.rs"]
mod protocol;
#[path = "core/regular.rs"]
mod regular;
pub use entry_error::EntryError;
pub use platform::{ConfinedDirectory, DirectoryHandle, QuarantinedChild};
pub use regular::{FinalLink, NotRegular};

// The analytics access tests (remote Windows task) reach the privileged
// fixtures through these names.
#[cfg(all(any(test, feature = "test-support"), windows))]
pub use platform::{access_fixtures, BackupRead};

#[cfg(test)]
#[path = "directory_handle_tests.rs"]
mod directory_handle_tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Link,
    #[default]
    Other,
}

#[derive(Default)]
pub struct LocalEntry {
    pub name: OsString,
    pub kind: EntryKind,
    pub is_dir: bool,
    pub is_link_like: bool,
    /// Logical size (end of file).
    pub size: u64,
    /// Bytes the file occupies on its volume (Windows `AllocationSize`,
    /// Unix `st_blocks * 512`); `None` when the platform path cannot say.
    /// Cloud placeholders, compressed, sparse and deduplicated files occupy
    /// less than `size`.
    pub allocated: Option<u64>,
    /// `(device, file id)` of a file that may have further hard links, so a
    /// walk counts its occupied bytes once: Unix only with a link count
    /// above one; Windows (link count unknown) for every file of the
    /// extended directory records that occupies space. `None` for a single
    /// link or when the platform path cannot say.
    pub hardlink: Option<(u64, u64)>,
    /// File system of the entry (Unix `st_dev`), for walks that stay on one
    /// file system; `None` where the listing did not stat the entry
    /// (Windows, folders of a size listing) — `DirectoryHandle::device_id`
    /// tells it for an opened folder.
    pub device: Option<u64>,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub hidden: bool,
    pub system: bool,
    pub unreachable: bool,
}

/// Link boundary and data-stream class of one entry's metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetadataClass {
    /// Symlink, junction or name-surrogate reparse point: a walk boundary.
    pub link_like: bool,
    /// FIFO, socket or device (Windows: device attribute, AF_UNIX/LX tags).
    pub special: bool,
}

pub use platform::{
    begin_scan, can_request_access, device_of, display_path, is_volume_mount_point, metadata_class,
    metadata_is_link_like, normalize_scan_root, parallel_scan_allowed, read_directory,
    request_access, run_helper_if_requested,
};

pub fn open_read(path: &Path) -> io::Result<File> {
    platform::open_read(path)
}

/// Open `path` for reading only if it is a regular file: it never waits on a
/// FIFO or reads a device, and refuses folders, special files and, with
/// `FinalLink::Refuse`, a link at the last component (`NotRegular` inside an
/// `InvalidInput` error). Windows data reparse points (cloud placeholders,
/// WOF, dedup) are regular files.
pub fn open_regular(path: &Path, final_link: FinalLink) -> io::Result<File> {
    platform::open_regular(path, final_link)
}

/// Classify the opened object itself, never a path that may have changed.
pub fn classify_open_file(file: &File) -> io::Result<MetadataClass> {
    platform::classify_open_file(file)
}

/// Harden only an object owned by the effective caller, before any private
/// bytes are read/written. Refuses redirects, special files and hardlinked
/// files. Windows migration requires READ_CONTROL and WRITE_DAC on the handle.
pub fn secure_private_handle(file: &File, is_directory: bool) -> io::Result<()> {
    platform::secure_private_handle(file, is_directory)
}

pub fn symlink_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    platform::symlink_metadata(path)
}

pub fn system_time_ms(time: std::time::SystemTime) -> i64 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as i64,
        Err(error) => -(error.duration().as_millis() as i64),
    }
}
