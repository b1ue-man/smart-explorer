pub(super) use crate::local_access::{
    display_path, normalize_scan_root, parallel_scan_allowed, read_directory, EntryKind, LocalEntry,
};

#[cfg(windows)]
#[path = "windows.rs"]
mod host;
#[cfg(target_os = "android")]
#[path = "android.rs"]
mod host;
#[cfg(all(not(windows), not(target_os = "android")))]
#[path = "linux_os.rs"]
mod host;
pub(crate) use host::host_recycle_available;
pub(crate) use host::{host_permission_note, recycle, volume_usage};

/// Default worker count of a local scan. Android's shared storage is served
/// by MediaProvider's multi-threaded FUSE daemon, whose readdirplus answers
/// fill the attribute cache, so directory listings scale with the cores; four
/// keep phones cool and responsive. The desktop keeps its established two.
#[cfg(target_os = "android")]
pub(super) fn default_scan_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |cores| cores.get().min(4))
}

#[cfg(not(target_os = "android"))]
pub(super) fn default_scan_threads() -> usize {
    2
}

#[path = "shared/checked_recycle.rs"]
mod checked_recycle;

#[cfg(target_os = "linux")]
#[path = "linux_trash.rs"]
mod linux_trash;
