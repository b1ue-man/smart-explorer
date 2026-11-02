//! Stage files of the local backend: the durable stage writer and finishing
//! a closed stage (source time, permission bits, flush).
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::local_platform;
use super::{StageDurability, StageFinish, StageFinished};

/// Permission bits a stage may receive (no set-id or sticky bits).
const STAGE_MODE_MASK: u32 = 0o777;

/// A stage that is on stable storage once flushed: the sized copy stage of
/// sync, mounts and Share hosts (`open_write_copy_stage_sized`).
pub(super) struct DurableFile(pub(super) std::fs::File);

impl Write for DurableFile {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()?;
        self.0.sync_all()
    }
}

/// Times, permission bits and flushing of a closed local stage. A stage on
/// `batched_device` (a local block filesystem the next `sync_filesystem`
/// flushes as a whole) with `Deferred` durability is not flushed on its own.
/// Storing the time or mode can be impossible on the target (FAT, Android
/// storage): that is reported or skipped, never an error; a failed flush is.
pub(super) fn finish_local_stage(
    path: &Path,
    finish: StageFinish,
    batched_device: Option<u64>,
) -> io::Result<StageFinished> {
    let mut finished = StageFinished::default();
    let durable = finish.durability != StageDurability::NotRequired;
    if finish.mtime_ms.is_none() && finish.mode.is_none() && !durable {
        return Ok(finished);
    }
    let file = local_platform::open_stage(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
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
    if durable {
        let deferred = finish.durability == StageDurability::Deferred
            && batched_device.is_some()
            && local_platform::device_of(&metadata) == batched_device;
        if !deferred {
            file.sync_all()?;
        }
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
