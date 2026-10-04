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
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        self.list_tolerant(path)
    }

    fn open_read_regular(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let file = local_access::open_regular(&local_platform::to_os(path), FinalLink::Refuse)?;
        Ok(Box::new(file))
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
