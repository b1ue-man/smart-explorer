//! P14 requires a namespace reservation, not a create/check/cleanup race.
//! Drive v3 `files.name` is not unique in a parent. Generated IDs deduplicate
//! our own create retry but cannot reserve a name against another client.
use super::GDriveBackend;
use crate::vfs::{Backend, VfsResult};
use std::io;

pub(super) fn create(_backend: &GDriveBackend, _path: &str) -> VfsResult<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported,
        "Google Drive permits duplicate names; atomic path-based folder reservation is unavailable"))
}

pub(super) fn identity(backend: &GDriveBackend, path: &str) -> VfsResult<Option<String>> {
    // sync_meta verifies the actual account/parent/title and retains the
    // existing object-bound locator. Never manufacture an ID from its name.
    let metadata = backend.sync_meta(path)?;
    if !metadata.is_dir || metadata.is_symlink || metadata.special {
        return Ok(None);
    }
    let namespace = backend.namespace_identity();
    Ok(metadata.id.filter(|id| !id.is_empty()).map(|id|
        format!("drive-folder:{}:{namespace}:{}:{id}", namespace.len(), id.len())))
}

/// The account/parent/title-checked Drive object ID survives its own rename.
pub(super) fn file_identity(backend: &GDriveBackend, path: &str) -> VfsResult<Option<String>> {
    let metadata = backend.sync_meta(path)?;
    if metadata.is_dir || metadata.is_symlink || metadata.special { return Ok(None); }
    let namespace = backend.namespace_identity();
    Ok(metadata.id.filter(|id| !id.is_empty()).map(|id|
        format!("drive-file:{}:{namespace}:{}:{id}", namespace.len(), id.len())))
}
