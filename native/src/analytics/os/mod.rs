pub(super) use crate::local_access::{
    display_path, normalize_scan_root, parallel_scan_allowed, read_directory, EntryKind, LocalEntry,
};

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
