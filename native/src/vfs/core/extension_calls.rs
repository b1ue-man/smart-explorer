//! Uniform calls of the optional backend extensions: each one uses the
//! backend's `BackendExtensions` when it has them and the documented
//! fallback otherwise, so callers never branch on the backend kind.
use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::extension_types::{
    ChangeNotice, ChangeSignalMode, ChangeSubscription, HashWalkItem, HashWalkRequest,
    MtimePrecision, RecycleExpectation, RecycleOutcome, StageFinish, StageFinished, TargetLimits,
    VfsListing,
};
use super::{Backend, VfsResult, VolumeIdentity};

pub(super) fn default_sync_child_path(parent: &str, literal_name: &str) -> VfsResult<String> {
    if literal_name.is_empty()
        || matches!(literal_name, "." | "..")
        || literal_name.contains('/')
        || literal_name.contains('\0')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sync child must be one literal path component",
        ));
    }
    Ok(format!("{}/{}", parent.trim_end_matches('/'), literal_name))
}

/// Encode only one new literal name, retaining the parent's locator meaning.
pub fn sync_child_path<B: Backend + ?Sized>(
    backend: &B,
    parent: &str,
    literal_name: &str,
) -> VfsResult<String> {
    // Every implementation receives a single validated component.
    default_sync_child_path(parent, literal_name)?;
    match backend.extensions() {
        Some(extensions) => extensions.sync_child_path(parent, literal_name),
        None => default_sync_child_path(parent, literal_name),
    }
}

/// Resolve literal relative sync components below an unchanged provider root.
pub fn sync_path<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
    literal_rel: &str,
) -> VfsResult<String> {
    let mut path = root.to_string();
    if !literal_rel.is_empty() {
        for name in literal_rel.split('/') {
            path = sync_child_path(backend, &path, name)?;
        }
    }
    Ok(path)
}

/// Reversible publication with a pre-journaled recovery sibling. No fallback
/// deletes or overwrites the original when the backend has no such operation.
pub fn replace_staged_reversible<B: Backend + ?Sized>(
    backend: &B,
    staged: &str,
    destination: &str,
    retained: &str,
) -> VfsResult<bool> {
    let (parent, name) = retained.rsplit_once('/').unwrap_or(("", retained));
    let destination_parent = destination.rsplit_once('/').map_or("", |(parent, _)| parent);
    let valid_nonce = name.rsplit_once(".se-replace-").is_some_and(|(base, nonce)| {
        !base.is_empty()
            && nonce.len() == 16
            && nonce.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if parent != destination_parent || !valid_nonce || retained == staged || retained == destination {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "replacement recovery path must be a unique generated sibling",
        ));
    }
    match backend.extensions() {
        Some(extensions) => extensions.replace_staged_reversible(staged, destination, retained),
        None => Ok(false),
    }
}

/// Listing that reports unlistable entries instead of failing the folder.
pub fn list_dir_tolerant<B: Backend + ?Sized>(backend: &B, path: &str) -> VfsResult<VfsListing> {
    match backend.extensions() {
        Some(extensions) => extensions.list_dir_tolerant(path),
        None => backend.list_dir(path).map(VfsListing::complete),
    }
}

/// Read a listed regular file without following a link or waiting on a
/// special file (local); other backends read as `open_read_id`.
pub fn open_read_regular<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    id: Option<&str>,
) -> VfsResult<Box<dyn Read + Send>> {
    match backend.extensions() {
        Some(extensions) => extensions.open_read_regular(path, id),
        None => backend.open_read_id(path, id),
    }
}

/// Sized copy stage that also carries the source time where the provider
/// can store it only while uploading.
pub fn open_write_copy_stage_timed<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    size: u64,
    mtime_ms: i64,
) -> VfsResult<Box<dyn Write + Send>> {
    match backend.extensions() {
        Some(extensions) => extensions.open_write_copy_stage_timed(path, size, mtime_ms),
        None => backend.open_write_copy_stage_sized(path, size),
    }
}

/// Source time and durability for a complete stage before publishing it.
pub fn finish_stage<B: Backend + ?Sized>(
    backend: &B,
    stage: &str,
    finish: StageFinish,
) -> VfsResult<StageFinished> {
    match backend.extensions() {
        Some(extensions) => extensions.finish_stage(stage, finish),
        None => Ok(StageFinished::default()),
    }
}

/// Flush deferred stages published below `root`; `Ok(false)` = unavailable.
pub fn sync_filesystem<B: Backend + ?Sized>(backend: &B, root: &str) -> VfsResult<bool> {
    match backend.extensions() {
        Some(extensions) => extensions.sync_filesystem(root),
        None => Ok(false),
    }
}

/// What the target below `root` can store (all unknown without extensions).
pub fn target_limits<B: Backend + ?Sized>(backend: &B, root: &str) -> TargetLimits {
    match backend.extensions() {
        Some(extensions) => extensions.target_limits(root),
        None => TargetLimits::default(),
    }
}

/// Modification-time resolution below `root` (`Unknown` without extensions).
pub fn mtime_precision<B: Backend + ?Sized>(backend: &B, root: &str) -> MtimePrecision {
    target_limits(backend, root).mtime_precision
}

/// Unix permission bits of `path` where the backend keeps them.
pub fn unix_mode<B: Backend + ?Sized>(backend: &B, path: &str) -> VfsResult<Option<u32>> {
    match backend.extensions() {
        Some(extensions) => extensions.unix_mode(path),
        None => Ok(None),
    }
}

/// Filesystem identity of `root`; `Ok(None)` = unknown (never "foreign").
pub fn volume_identity<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
) -> VfsResult<Option<VolumeIdentity>> {
    match backend.extensions() {
        Some(extensions) => extensions.volume_identity(root),
        None => Ok(None),
    }
}

pub fn supports_duplicate_search<B: Backend + ?Sized>(backend: &B, root: &str) -> VfsResult<bool> {
    match backend.extensions() {
        Some(extensions) => extensions.supports_duplicate_search(root),
        None => Ok(false),
    }
}

/// Host-side duplicate search; `Ok(None)` = unsupported (search elsewhere).
pub fn find_duplicates<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
    min_bytes: u64,
    progress: &crate::analytics::ReclaimProgress,
) -> VfsResult<Option<crate::analytics::DuplicateReport>> {
    match backend.extensions() {
        Some(extensions) => extensions.find_duplicates(root, min_bytes, progress),
        None => Ok(None),
    }
}

pub fn supports_hash_walk<B: Backend + ?Sized>(backend: &B, root: &str) -> VfsResult<bool> {
    match backend.extensions() {
        Some(extensions) => extensions.supports_hash_walk(root),
        None => Ok(false),
    }
}

/// Host-side hash walk; `Ok(false)` = unsupported before any item.
pub fn hash_walk<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
    request: HashWalkRequest,
    tx: crossbeam_channel::Sender<HashWalkItem>,
    cancel: &AtomicBool,
) -> VfsResult<bool> {
    match backend.extensions() {
        Some(extensions) => extensions.hash_walk(root, request, tx, cancel),
        None => Ok(false),
    }
}

pub fn supports_recycle<B: Backend + ?Sized>(backend: &B, path: &str) -> VfsResult<bool> {
    match backend.extensions() {
        Some(extensions) => extensions.supports_recycle(path),
        None => Ok(false),
    }
}

/// Trash on the storing device after re-checking the content.
pub fn recycle<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    expected: &RecycleExpectation,
) -> VfsResult<RecycleOutcome> {
    match backend.extensions() {
        Some(extensions) => extensions.recycle(path, expected),
        None => super::meta::unsupported("recycling on the storing device is not supported"),
    }
}

pub fn change_signal_mode<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
) -> VfsResult<Option<ChangeSignalMode>> {
    match backend.extensions() {
        Some(extensions) => extensions.change_signal_mode(root),
        None => Ok(None),
    }
}

/// Change subscription for `root`; `Ok(None)` = unsupported (poll yourself).
pub fn change_signal<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
    poll_interval: Duration,
    tx: crossbeam_channel::Sender<ChangeNotice>,
) -> VfsResult<Option<ChangeSubscription>> {
    match backend.extensions() {
        Some(extensions) => extensions.change_signal(root, poll_interval, tx),
        None => Ok(None),
    }
}
