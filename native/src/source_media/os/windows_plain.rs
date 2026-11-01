//! "Not optical" answers that a probe of a drive path takes from the folder
//! above it instead of resolving the volume again (2026-11-01).
//!
//! A full probe (`windows.rs` `resolve`) resolves the volume path
//! (`GetVolumePathNameW`) and the drive type, and on an optical drive the
//! device. Every local listing and every local status of the `vfs` local
//! backend probes its path (`vfs/os/shared/local_optical.rs`), and so does
//! every guarded read (`reader.rs` `guard_reader`): a sync walk of a local
//! side probes each folder twice, the checksum mode each file five times.
//! Measured cost of one full probe, suite run 41253633650, job
//! `measure (windows-2025)`, warm, 4 CPUs: `se ls -R` of 100 000 files in
//! 10 000 folders took 46.2 s while the scanner probed every folder and every
//! entry, 0.46 s without (`walk_device.rs`), about 0.38 ms per probe.
//!
//! A path that is no reparse point is no mount point: a volume mounted in a
//! folder makes that folder a reparse point (`IO_REPARSE_TAG_MOUNT_POINT`),
//! and a drive root has no folder above it. Such a path lies on the volume of
//! its parent folder, so the parent's "not optical" answer is its answer too.
//! It is taken only while that answer stands (`probe_cache::FRESH`) and passed
//! on with its time, so every derived answer rests on a full probe at most
//! `FRESH` old, as the answers kept for files already did (M6). Only "not
//! optical" is ever kept: an optical answer is resolved in full every time.
//! Resolved in full as before: reparse points (junctions, links, mount
//! points, cloud placeholders), paths whose attributes cannot be read, and
//! paths below no drive letter (`\\?\Volume{…}`, `\\?\GLOBALROOT\…`, where a
//! component can name a device). Keys are the exact verbatim text, so another
//! spelling of a folder is resolved again rather than taken from a folder that
//! only looks alike (case-sensitive folders).
//!
//! A folder that turns into a mount point between its attribute read and its
//! full probe passes the answer of the mounted volume to its parent; only a
//! parent on an optical volume would be wrong then, and optical file systems
//! take no new mount points.
use super::super::probe_cache;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, Prefix};
use std::time::Instant;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    INVALID_FILE_ATTRIBUTES,
};

#[cfg(test)]
#[path = "windows_plain_tests.rs"]
mod tests;

/// The probe of the verbatim drive or volume path `path`: a "not optical"
/// answer kept for it or taken from its parent folder, else `resolve(path)`.
pub(super) fn probe_known<T>(
    path: &Path,
    resolve: impl FnOnce(&Path) -> io::Result<Option<T>>,
) -> io::Result<Option<T>> {
    let own = key(path);
    if probe_cache::plain_since(&own).is_some() {
        return Ok(None);
    }
    let attributes = attributes(path);
    let directory = attributes.is_some_and(|value| value & FILE_ATTRIBUTE_DIRECTORY != 0);
    let parent = attributes
        .filter(|value| value & FILE_ATTRIBUTE_REPARSE_POINT == 0)
        .and_then(|_| drive_parent(path))
        .map(key);
    if let Some(parent) = &parent {
        if let Some(since) = probe_cache::plain_since(parent) {
            // Kept for folders only: the paths below them ask for it, a
            // file's own answer is never asked by another path.
            if directory {
                probe_cache::remember_plain_at(own, since);
            }
            return Ok(None);
        }
    }
    let asked = Instant::now();
    let found = resolve(path)?;
    if found.is_none() {
        if let Some(parent) = parent {
            probe_cache::remember_plain_at(parent, asked);
        }
        if directory {
            probe_cache::remember_plain_at(own, asked);
        }
    }
    Ok(found)
}

fn key(path: &Path) -> String {
    format!("probe:{}", path.to_string_lossy())
}

/// The attributes of `path` itself (a link as the link); `None` when they
/// cannot be read.
fn attributes(path: &Path) -> Option<u32> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let value = unsafe { GetFileAttributesW(wide.as_ptr()) };
    (value != INVALID_FILE_ATTRIBUTES).then_some(value)
}

/// The folder above `path` when `path` is a verbatim drive path below the
/// drive root (`\\?\C:\a` gives `\\?\C:\`, the root itself gives nothing).
fn drive_parent(path: &Path) -> Option<&Path> {
    match path.components().next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_)) => {
            path.parent()
        }
        _ => None,
    }
}

/// Test hooks: full probes counted below watched folders, and a switch that
/// resolves every probe in full (the answers before this change).
#[cfg(test)]
pub(super) mod hooks {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    static ALWAYS: AtomicBool = AtomicBool::new(false);
    static WATCHED: Mutex<BTreeMap<PathBuf, usize>> = Mutex::new(BTreeMap::new());

    pub(in super::super) fn resolving_always() -> bool {
        ALWAYS.load(Ordering::SeqCst)
    }

    /// While set, every probe of this process is resolved in full. Other
    /// tests then only take the full probe they took before this change.
    pub(in super::super) fn resolve_always(on: bool) {
        ALWAYS.store(on, Ordering::SeqCst);
    }

    pub(in super::super) fn count_resolution(path: &Path) {
        let mut watched = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
        for (root, count) in watched.iter_mut() {
            if path.starts_with(root) {
                *count += 1;
            }
        }
    }

    /// Counts the full probes of paths below `root` while it lives.
    pub(in super::super) struct Resolutions(PathBuf);

    pub(in super::super) fn watch(root: &Path) -> Resolutions {
        let root = crate::local_access::normalize_scan_root(root);
        WATCHED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(root.clone(), 0);
        Resolutions(root)
    }

    impl Resolutions {
        pub(in super::super) fn count(&self) -> usize {
            WATCHED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&self.0)
                .copied()
                .unwrap_or(0)
        }
    }

    impl Drop for Resolutions {
        fn drop(&mut self) {
            WATCHED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.0);
        }
    }
}
