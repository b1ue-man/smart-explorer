//! Host-backed spool for providers that cannot overlap a reader and writer.
use super::Backend;
use std::io::{self, Read, Seek, SeekFrom, Write};

pub(crate) fn copy_file<B: Backend + ?Sized>(
    backend: &B, source: &str, destination: &str,
) -> io::Result<u64> {
    copy_between(backend, source, backend, destination)
}

pub(crate) fn copy_between<S: Backend + ?Sized, D: Backend + ?Sized>(
    source_backend: &S, source: &str, destination_backend: &D, destination: &str,
) -> io::Result<u64> {
    let before = source_backend.stat(source)?;
    if before.is_dir || before.is_symlink {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "copy source is not a regular file"));
    }
    // Complete reading before opening a writer on a potentially single-session
    // backend. The disk spool bounds RAM; one extra byte detects source growth.
    let mut spool = tempfile::tempfile()?;
    let read_size = source_backend.read_size(source, before.size)?;
    let mut reader = source_backend.open_read_id(source, before.id.as_deref())?
        .take(read_size.map(|size| size.saturating_add(1)).unwrap_or(u64::MAX));
    let copied = io::copy(&mut reader, &mut spool)?;
    drop(reader);
    let after = source_backend.stat(source)?;
    if read_size.is_some_and(|size| copied != size) || after.is_dir || after.is_symlink
        || after.size != before.size || after.mtime_ms != before.mtime_ms
        || after.id != before.id || after.content_md5 != before.content_md5
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "copy source changed during transfer"));
    }
    spool.seek(SeekFrom::Start(0))?;
    let staged = super::unique_staging_path(destination_backend, destination, "copy")?;
    let result = (|| {
        let mut writer = destination_backend.open_write_copy_stage(&staged)?;
        io::copy(&mut spool, &mut writer)?;
        writer.flush()?;
        drop(writer);
        super::promote_staged_replace(destination_backend, &staged, destination)?;
        Ok(copied)
    })();
    result.map_err(|error: io::Error| io::Error::new(
        error.kind(), format!("{error}; unconfirmed copy stage retained if present: {staged}"),
    ))
}
