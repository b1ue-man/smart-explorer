//! Where a document reads its bytes: any source that answers its length and
//! a read at an offset. Local files, every remote backend (through
//! `vfs::Backend`, see `VfsSource`) and test data in memory implement it, so
//! a document never holds the whole file in memory.
use std::io;
use std::sync::Arc;

/// Random-access bytes of one file.
pub trait TextSource: Send + Sync {
    /// Length in bytes when the document was opened.
    fn len(&self) -> u64;

    /// Whether the source has no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads up to `buf.len()` bytes at `offset`; fewer only at the end of the
    /// source or when the transport returns a short read (callers loop).
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;
}

impl<T: TextSource + ?Sized> TextSource for Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        (**self).read_at(offset, buf)
    }
}

/// A source over bytes in memory (tests, previews of small content).
#[derive(Clone, Debug, Default)]
pub struct BytesSource {
    bytes: Arc<[u8]>,
}

impl BytesSource {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: Arc::from(bytes.into()),
        }
    }
}

impl TextSource for BytesSource {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let Ok(start) = usize::try_from(offset) else {
            return Ok(0);
        };
        if start >= self.bytes.len() {
            return Ok(0);
        }
        let n = buf.len().min(self.bytes.len() - start);
        buf[..n].copy_from_slice(&self.bytes[start..start + n]);
        Ok(n)
    }
}

/// Fills `buf` from `offset` until it is full or the source ends; returns the
/// number of bytes read. An interrupted read is retried.
pub(crate) fn read_full_at(
    source: &dyn TextSource,
    offset: u64,
    buf: &mut [u8],
) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match source.read_at(offset + filled as u64, &mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}
