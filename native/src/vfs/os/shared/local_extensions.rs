//! Optional extensions of the local disk backend, the local volume identity
//! and mount boundaries.
use std::io::{self, Read, Write};

use super::fs_profile::FlushModel;
use super::local_platform;
use super::local_stage::finish_local_stage;
use super::{
    BackendExtensions, LocalBackend, MountKind, StageFinish, StageFinished, TargetLimits,
    VfsListing, VfsResult, VolumeIdentity,
};
use crate::local_access::{self, FinalLink};

impl BackendExtensions for LocalBackend {
    fn confined_local_root(&self, root: &str) -> VfsResult<Option<super::BackendHandle>> {
        super::ConfinedLocalBackend::open(root)
            .map(|backend| Some(std::sync::Arc::new(backend) as super::BackendHandle))
    }

    fn directory_reservation_supported(&self, _root: &str) -> bool {
        true
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        local_platform::check_new_name(&local_platform::to_os(path))?;
        let (parent, name) = confined_parent(path)?;
        parent.create_dir(&name).map(|_| ())
    }

    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        match local_directory_identity(path) {
            Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(None),
            result => result,
        }
    }

    fn file_identity(&self, path: &str) -> VfsResult<Option<String>> {
        super::local_file_identity::local_file_identity(path)
    }

    fn capacity(&self, root: &str) -> VfsResult<crate::vfs::CapacitySnapshot> {
        let identity = crate::vfs::Backend::state_identity(self);
        match super::local_capacity::query(root) {
            Ok(mut value) => {
                value.identity = identity;
                Ok(value)
            }
            Err(error) => Ok(crate::vfs::capacity_failure(
                identity,
                root,
                crate::vfs::CapacitySource::Filesystem,
                &error,
            )),
        }
    }

    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        self.list_tolerant(path)
    }

    /// Hashed here: the file is on this device.
    fn content_digest(
        &self,
        path: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<super::ContentDigest>> {
        super::digest_by_local_read(self, path, cancel)
    }

    fn open_read_regular(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        crate::source_media::guard_reader(&local_platform::to_os(path), || {
            local_access::open_regular(&local_platform::to_os(path), FinalLink::Refuse)
        })
    }

    /// A new stage only the owner can read until `finish_stage` gives it its
    /// final mode; its time is set when it is finished.
    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        _size: u64,
        _mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(local_platform::create_new_private(
            &local_platform::to_os(path),
        )?))
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        finish_local_stage(&local_platform::to_os(stage), finish, self.batched_device())
    }

    /// Local block filesystems and NFS: one `syncfs` makes every deferred
    /// stage and every publishing rename durable (Windows: stages are flushed
    /// one by one and published by write-through renames). FUSE, SMB mounts
    /// and unknown types give no guarantee for renames.
    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        let path = local_platform::to_os(root);
        let flush = local_platform::filesystem_profile(&path)
            .map_or(FlushModel::PerFileOnly, |profile| profile.flush);
        if flush == FlushModel::PerFileOnly {
            return Ok(false);
        }
        local_platform::flush_filesystem(&path)?;
        Ok(true)
    }

    fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
        local_platform::confirm_namespace(&local_platform::to_os(parent))
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        local_platform::filesystem_profile(&local_platform::to_os(root))
            .map(|profile| profile.limits)
            .unwrap_or_else(|_| local_platform::fallback_limits())
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        let metadata = local_access::symlink_metadata(&local_platform::to_os(path))?;
        Ok(local_platform::unix_mode(&metadata))
    }

    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        local_volume_identity(root)
    }
}

/// Filesystem identity of a local location: filesystem UUID or volume
/// serial plus the location inside that filesystem, independent of mount
/// point and drive letter. `Ok(None)` = not determinable ("unknown", never
/// "another volume").
pub fn local_volume_identity(path: &str) -> io::Result<Option<VolumeIdentity>> {
    local_platform::volume_identity(&local_platform::to_os(path))
}

/// The kind of filesystem mounted at the local directory `path` when it is
/// a mount point inside its parent's tree; `Ok(None)` = same filesystem as
/// the parent (Windows: volumes mounted in folders are junctions, i.e. links).
pub fn local_mount_boundary(path: &str) -> io::Result<Option<MountKind>> {
    local_platform::mount_boundary(&local_platform::to_os(path))
}

fn confined_parent(
    path: &str,
) -> io::Result<(crate::local_access::ConfinedDirectory, std::ffi::OsString)> {
    let path = local_platform::to_os(path);
    confined_parent_os(&path)
}
fn confined_parent_os(
    path: &std::path::Path,
) -> io::Result<(crate::local_access::ConfinedDirectory, std::ffi::OsString)> {
    let name = path
        .file_name()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "root is not one child directory",
            )
        })?
        .to_os_string();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    Ok((
        crate::local_access::ConfinedDirectory::open_root(parent)?,
        name,
    ))
}

fn local_directory_identity(path: &str) -> io::Result<Option<String>> {
    let path = local_platform::to_os(path);
    if path.file_name().is_none() {
        return crate::local_access::ConfinedDirectory::open_root(&path)?
            .identity()
            .map(Some);
    }
    let (parent, name) = confined_parent_os(&path)?;
    let entry = parent.selected_entry(&name)?;
    if !entry.metadata()?.is_dir() || local_access::classify_open_file(&entry)?.link_like {
        return Ok(None);
    }
    parent.select_child_root(&name)?.stable_identity()
}
