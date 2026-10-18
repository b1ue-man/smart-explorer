//! Host-backed spool for providers that cannot overlap a reader and writer.
//! With a vault on either side the spool holds the bytes only encrypted
//! under a key used once (K08 of the vault plan).
use super::{Backend, Scheme};
use std::io::{self, Read, Seek, SeekFrom, Write};

pub(crate) fn copy_file<B: Backend + ?Sized>(
    backend: &B,
    source: &str,
    destination: &str,
) -> io::Result<u64> {
    copy_between(backend, source, backend, destination)
}

/// The spool of one copy: a plain anonymous file, or an encrypted one when a
/// vault takes part.
enum Spool {
    Plain(std::fs::File),
    Sealed(crate::vault::EphemeralSpool),
}

impl Spool {
    fn new(sealed: bool) -> io::Result<Self> {
        if sealed {
            Ok(Spool::Sealed(crate::vault::EphemeralSpool::new()?))
        } else {
            Ok(Spool::Plain(tempfile::tempfile()?))
        }
    }

    fn fill(&mut self, reader: &mut dyn Read) -> io::Result<u64> {
        match self {
            Spool::Plain(file) => io::copy(reader, file),
            Spool::Sealed(spool) => spool.fill(reader),
        }
    }

    fn drain(&mut self, writer: &mut dyn Write) -> io::Result<u64> {
        match self {
            Spool::Plain(file) => {
                file.seek(SeekFrom::Start(0))?;
                io::copy(file, writer)
            }
            Spool::Sealed(spool) => io::copy(&mut spool.reader()?, writer),
        }
    }
}

pub(crate) fn copy_between<S: Backend + ?Sized, D: Backend + ?Sized>(
    source_backend: &S,
    source: &str,
    destination_backend: &D,
    destination: &str,
) -> io::Result<u64> {
    let before = source_backend.stat(source)?;
    if before.is_dir || before.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "copy source is not a regular file",
        ));
    }
    // Complete reading before opening a writer on a potentially single-session
    // backend. The disk spool bounds RAM; one extra byte detects source growth.
    let vault =
        source_backend.scheme() == Scheme::Vault || destination_backend.scheme() == Scheme::Vault;
    let mut spool = Spool::new(vault)?;
    let read_size = source_backend.read_size(source, before.size)?;
    let mut reader = source_backend
        .open_read_id(source, before.id.as_deref())?
        .take(
            read_size
                .map(|size| size.saturating_add(1))
                .unwrap_or(u64::MAX),
        );
    let copied = spool.fill(&mut reader)?;
    drop(reader);
    let after = source_backend.stat(source)?;
    if read_size.is_some_and(|size| copied != size)
        || after.is_dir
        || after.is_symlink
        || after.size != before.size
        || after.mtime_ms != before.mtime_ms
        || after.id != before.id
        || after.content_md5 != before.content_md5
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "copy source changed during transfer",
        ));
    }
    let staged = super::unique_staging_path(destination_backend, destination, "copy")?;
    let result = (|| {
        let mut writer = destination_backend.open_write_copy_stage(&staged)?;
        spool.drain(&mut writer)?;
        writer.flush()?;
        drop(writer);
        super::promote_staged_replace(destination_backend, &staged, destination)?;
        Ok(copied)
    })();
    result.map_err(|error: io::Error| {
        io::Error::new(
            error.kind(),
            format!("{error}; unconfirmed copy stage retained if present: {staged}"),
        )
    })
}
