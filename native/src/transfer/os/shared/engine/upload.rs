//! Local file → remote target: the source is observed before and after the
//! copy and read through `local_access` (Win32-hostile names, consented
//! protected reads); the bytes go to a fresh object or a private stage and
//! are published create-only.
use super::ops::{At, OpError, OpResult, Outcome};
use super::publish;
use super::queue::FileWork;
use super::source::LocalSource;
use super::Engine;
use crate::vfs::Backend;

pub(super) fn upload(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    parent_created: bool,
    meter: &super::ops::Meter<'_>,
    buffer: &mut [u8],
) -> OpResult {
    let mut source = LocalSource::open(&file.source)?;
    let size = source.length();
    if engine.resume {
        if let Some(outcome) = resume_check(engine, target, file, size)? {
            return Ok(outcome);
        }
    }
    let mut destination = publish::open(engine, target, file, Some(size), parent_created)?;
    let mut copied = 0u64;
    loop {
        if engine.stopped() {
            publish::abandon(engine, target, destination);
            return Err(OpError::canceled());
        }
        let read = match source.read_bounded(buffer, copied) {
            Ok(read) => read,
            Err(error) => {
                publish::abandon(engine, target, destination);
                return Err(error);
            }
        };
        if read == 0 {
            break;
        }
        if let Err(error) = destination.writer().write_all(&buffer[..read]) {
            publish::abandon(engine, target, destination);
            return Err(OpError::target(error));
        }
        copied += read as u64;
        meter.add(read as u64);
    }
    if let Err(error) = source.complete(copied) {
        publish::abandon(engine, target, destination);
        return Err(error);
    }
    publish::complete(engine, target, file, destination, || source.verify())
}

/// "Transfer missing files": an existing destination of the same size is
/// kept and counted as skipped, any other stays untouched (K8).
pub(super) fn resume_check(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    size: u64,
) -> Result<Option<Outcome>, OpError> {
    let path = engine.folders.path_of(&file.rel);
    match target.stat(&path) {
        Ok(meta) if !meta.is_dir && !meta.is_symlink && meta.size == size => {
            Ok(Some(Outcome::Skipped))
        }
        Ok(_) => Err(OpError::at(At::Publish, publish::different_size())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(OpError::target(error)),
    }
}
