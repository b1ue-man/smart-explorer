//! Immutable cache authority, written before the first host can create data.
//! Reading and exporting it never resolve a provider or reinterpret old bytes.
use crate::local_access::{DirectoryHandle, EntryKind};
use crate::mount::recovery_identity::{self, RecoveryBackendIdentity, IDENTITY_FILE};
use crate::mount::{MountConfig, MountId};
use std::{ffi::OsStr, io::{self, Write}, path::Path};

pub(crate) fn read_mount_recovery_identity(base: &Path, config: &MountConfig)
    -> io::Result<Option<RecoveryBackendIdentity>> {
    let directory = match open_cache(base, &config.id)? {
        Some(directory) => directory,
        None => return Ok(None),
    };
    let file = match directory.open_regular_child(OsStr::new(IDENTITY_FILE)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let bytes = super::recovery_export_io::read_bounded(file)?;
    let identity: RecoveryBackendIdentity = serde_json::from_slice(&bytes)
        .map_err(|_| recovery_identity::conflict())?;
    identity.validate_for(&config.source)?;
    Ok(Some(identity))
}

/// An absent proof may be established only before the cache contains any
/// journal history or payload. Never infer an old account from a new login.
pub(crate) fn preflight_mount_recovery_identity(base: &Path, config: &MountConfig)
    -> io::Result<Option<RecoveryBackendIdentity>> {
    let identity = read_mount_recovery_identity(base, config)?;
    if identity.is_none() {
        if let Some(directory) = open_cache(base, &config.id)? {
            require_empty_cache(&directory)?;
        }
    }
    Ok(identity)
}

pub(crate) fn bind_mount_recovery_identity(base: &Path, config: &MountConfig,
    identity: &RecoveryBackendIdentity, _lease: &crate::mount::RecoveryCacheLease) -> io::Result<()> {
    identity.validate_for(&config.source)?;
    if let Some(stored) = read_mount_recovery_identity(base, config)? {
        return if stored == *identity { Ok(()) } else { Err(recovery_identity::conflict()) };
    }
    let parent = DirectoryHandle::open_root(base)?;
    let directory = match parent.open_child(OsStr::new(config.id.as_str())) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let pending = format!(".identity-cache-{}.pending", MountId::new_random()?);
            drop(parent.create_private_child(OsStr::new(&pending))?);
            super::recovery_storage::publish(&base.join(pending), &base.join(config.id.as_str()), false)?;
            parent.open_child(OsStr::new(config.id.as_str()))?
        }
        Err(error) => return Err(error),
    };
    require_empty_cache(&directory)?;
    directory.secure_private()?;
    let pending = format!(".recovery-identity-{}.pending", MountId::new_random()?);
    let mut file = directory.create_file_new(OsStr::new(&pending))?;
    file.write_all(&serde_json::to_vec(identity).map_err(io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    let root = base.join(config.id.as_str());
    super::recovery_storage::publish(&root.join(pending), &root.join(IDENTITY_FILE), false)?;
    if read_mount_recovery_identity(base, config)?.as_ref() != Some(identity) {
        return Err(recovery_identity::conflict());
    }
    Ok(())
}

fn open_cache(base: &Path, id: &MountId) -> io::Result<Option<DirectoryHandle>> {
    let root = match DirectoryHandle::open_root(base) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    match root.open_child(OsStr::new(id.as_str())) {
        Ok(directory) => Ok(Some(directory)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn require_empty_cache(directory: &DirectoryHandle) -> io::Result<()> {
    for entry in directory.read_directory()? {
        let entry = entry?;
        if entry.is_link_like { return Err(recovery_identity::missing()); }
        match (entry.name.to_str(), entry.kind) {
            (Some("files"), EntryKind::Directory) => {
                if directory.open_child(&entry.name)?.read_directory()?.next().is_some() {
                    return Err(recovery_identity::missing());
                }
            }
            (Some("journal.jsonl"), EntryKind::File)
                if directory.open_regular_child(&entry.name)?.metadata()?.len() == 0 => {},
            // A crash before the first identity rename cannot have produced
            // mounted data. This private metadata stage grants no authority;
            // it may coexist with a retry only while journal/files are empty.
            (Some(name), EntryKind::File) if pending_identity_name(name) => {},
            _ => return Err(recovery_identity::missing()),
        }
    }
    Ok(())
}

fn pending_identity_name(name: &str) -> bool {
    name.strip_prefix(".recovery-identity-")
        .and_then(|name| name.strip_suffix(".pending"))
        .is_some_and(|id| MountId::parse(id).is_ok())
}
