use crate::vfs::{Backend, VfsMeta};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// Streaming block of one copy: large enough that per-call overhead of
/// remote readers and writers stays small, small enough that the flows'
/// highest concurrency (256 operations) holds at most 64 MiB of buffers.
const COPY_BUFFER: usize = 256 * 1024;

/// Copies one file into a private stage next to `destination_path` and
/// publishes it (create when `destination_expected` is `None`, replace
/// otherwise). The destination folder exists already: the mirror pass
/// creates every folder once before it queues the files inside. `progress`
/// receives every streamed block.
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_stream(
    source: &dyn Backend,
    source_path: &str,
    source_expected: &VfsMeta,
    destination: &dyn Backend,
    destination_path: &str,
    destination_expected: Option<&VfsMeta>,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> io::Result<u64> {
    let staged = crate::vfs::unique_staging_path(destination, destination_path, "sync")?;
    let result = (|| {
        let source_before = source.stat(source_path)?;
        validate_unchanged_source(source_path, source_expected, &source_before, "before")?;
        let mut reader = source.open_read(source_path)?;
        let mut writer = destination.open_write(&staged)?;
        let mut copied = 0u64;
        let mut buffer = vec![0u8; COPY_BUFFER];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "sync canceled"));
            }
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read])?;
            copied = copied.saturating_add(read as u64);
            progress(read as u64);
        }
        writer.flush()?;
        drop(writer);
        let source_after = source.stat(source_path)?;
        validate_unchanged_source(source_path, &source_before, &source_after, "during")?;
        if copied != source_before.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("source length changed during sync copy: {source_path}"),
            ));
        }
        validate_destination(destination, destination_path, destination_expected)?;
        if destination_expected.is_some() {
            crate::vfs::promote_staged_replace(destination, &staged, destination_path)?;
        } else {
            crate::vfs::promote_staged_create(destination, &staged, destination_path)?;
        }
        Ok(copied)
    })();
    if result.is_err() {
        let _ = destination.remove_file(&staged);
    }
    result
}

fn validate_unchanged_source(
    path: &str,
    expected: &VfsMeta,
    actual: &VfsMeta,
    phase: &str,
) -> io::Result<()> {
    if actual.is_dir
        || actual.is_symlink
        || actual.size != expected.size
        || actual.mtime_ms != expected.mtime_ms
        || matches!((&actual.id, &expected.id), (Some(a), Some(b)) if a != b)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("source changed {phase} sync copy: {path}"),
        ));
    }
    Ok(())
}

fn validate_destination(
    destination: &dyn Backend,
    path: &str,
    expected: Option<&VfsMeta>,
) -> io::Result<()> {
    match (expected, destination.stat(path)) {
        (None, Err(error)) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        (None, Ok(_)) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination appeared during sync copy: {path}"),
        )),
        (None, Err(error)) => Err(error),
        (Some(expected), Ok(actual))
            if !actual.is_dir
                && !actual.is_symlink
                && actual.size == expected.size
                && actual.mtime_ms == expected.mtime_ms
                && !matches!((&actual.id, &expected.id), (Some(a), Some(b)) if a != b) =>
        {
            Ok(())
        }
        (Some(_), Ok(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("destination changed during sync copy: {path}"),
        )),
        (Some(_), Err(error)) => Err(error),
    }
}
