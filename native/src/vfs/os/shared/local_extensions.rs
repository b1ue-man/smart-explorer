//! Optional extensions of the local disk backend and the local volume
//! identity.
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::local_platform;
use super::{
    BackendExtensions, LocalBackend, MountKind, StageDurability, StageFinish, StageFinished,
    VfsResult, VolumeIdentity,
};
use crate::local_access::{self, FinalLink};

/// Permission bits a stage may receive (no set-id or sticky bits).
const STAGE_MODE_MASK: u32 = 0o777;

impl BackendExtensions for LocalBackend {
    fn open_read_regular(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let file = local_access::open_regular(&local_platform::to_os(path), FinalLink::Refuse)?;
        Ok(Box::new(file))
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        finish_local_stage(&local_platform::to_os(stage), finish)
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        local_platform::syncfs(&local_platform::to_os(root))
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        let metadata = local_access::symlink_metadata(&local_platform::to_os(path))?;
        Ok(local_platform::unix_mode(&metadata))
    }

    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        local_volume_identity(root)
    }
}

/// Times, permission bits and flushing of a closed local stage. Storing the
/// time or mode can be impossible on the target (FAT, Android storage):
/// that is reported or skipped, never an error. A failed flush is an error.
fn finish_local_stage(path: &Path, finish: StageFinish) -> io::Result<StageFinished> {
    let mut finished = StageFinished::default();
    let flush = finish.durability != StageDurability::NotRequired;
    if finish.mtime_ms.is_none() && finish.mode.is_none() && !flush {
        return Ok(finished);
    }
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stage is not a regular file",
        ));
    }
    if let Some(mtime_ms) = finish.mtime_ms {
        finished.mtime_applied =
            match system_time(mtime_ms).and_then(|time| file.set_modified(time)) {
                Ok(()) => true,
                Err(error) if not_storable(&error) => false,
                Err(error) => return Err(error),
            };
    }
    if let Some(mode) = finish.mode {
        if let Err(error) = local_platform::set_unix_mode(&file, mode & STAGE_MODE_MASK) {
            if !not_storable(&error) {
                return Err(error);
            }
        }
    }
    if flush {
        // Deferred stages are flushed at once until the batched
        // `sync_filesystem` path knows which filesystems it covers.
        file.sync_all()?;
        finished.durable = true;
    }
    Ok(finished)
}

/// The target cannot store this metadata (no error of the stage itself).
fn not_storable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
    )
}

fn system_time(mtime_ms: i64) -> io::Result<SystemTime> {
    let offset = Duration::from_millis(mtime_ms.unsigned_abs());
    let time = if mtime_ms >= 0 {
        UNIX_EPOCH.checked_add(offset)
    } else {
        UNIX_EPOCH.checked_sub(offset)
    };
    time.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "modification time out of range",
        )
    })
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
