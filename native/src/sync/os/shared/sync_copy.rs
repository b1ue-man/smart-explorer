use crate::vfs::{Backend, VfsMeta};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// Streaming block of one copy: large enough that per-call overhead of
/// remote readers and writers stays small, small enough that the flows'
/// highest concurrency (256 operations) holds at most 64 MiB of buffers.
const COPY_BUFFER: usize = 256 * 1024;

/// A failed copy and how far it got.
pub(super) struct CopyError {
    pub(super) error: io::Error,
    /// Publication was attempted: its outcome is unknown, so the copy is
    /// never repeated (a repeat could publish a second time).
    pub(super) publishing: bool,
}

/// Copies one file into a private stage next to `destination_path` and
/// publishes it (create when `destination_expected` is `None`, replace
/// otherwise). The destination folder exists already: the mirror pass
/// creates every folder once before it queues the files inside. With
/// `guard_parent` (a local destination) the whole parent chain is checked
/// for links and junctions right before the stage is written and again right
/// before publishing, as the serial mirror's per-file `mkdir_all` did
/// (`LocalBackend::mkdir_all` refuses them on every level). `progress`
/// receives every streamed block.
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_stream(
    source: &dyn Backend,
    source_path: &str,
    source_expected: &VfsMeta,
    destination: &dyn Backend,
    destination_path: &str,
    destination_expected: Option<&VfsMeta>,
    guard_parent: bool,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<u64, CopyError> {
    let staged = crate::vfs::unique_staging_path(destination, destination_path, "sync").map_err(
        |error| CopyError {
            error,
            publishing: false,
        },
    )?;
    let prepared = (|| {
        let source_before = source.stat(source_path)?;
        validate_unchanged_source(source_path, source_expected, &source_before, "before")?;
        let mut reader = source.open_read(source_path)?;
        if guard_parent {
            plain_parent(destination, destination_path)?;
        }
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
        if guard_parent {
            plain_parent(destination, destination_path)?;
        }
        Ok(copied)
    })();
    let copied = match prepared {
        Ok(copied) => copied,
        Err(error) => {
            let _ = destination.remove_file(&staged);
            return Err(CopyError {
                error,
                publishing: false,
            });
        }
    };
    let published = if destination_expected.is_some() {
        crate::vfs::promote_staged_replace(destination, &staged, destination_path)
    } else {
        crate::vfs::promote_staged_create(destination, &staged, destination_path)
    };
    if let Err(error) = published {
        let _ = destination.remove_file(&staged);
        return Err(CopyError {
            error,
            publishing: true,
        });
    }
    Ok(copied)
}

/// The destination's parent chain consists of plain folders (created when
/// missing); a link or junction anywhere in it is refused.
fn plain_parent(destination: &dyn Backend, path: &str) -> io::Result<()> {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => destination.mkdir_all("/"),
        Some(index) => destination.mkdir_all(&trimmed[..index]),
        None => Ok(()),
    }
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
