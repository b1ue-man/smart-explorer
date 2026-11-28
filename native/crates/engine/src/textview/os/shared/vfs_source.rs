//! [`TextSource`] over a `vfs::Backend` path, so remote files are shown
//! without downloading them whole. Backends with range reads answer each
//! read with one range request; small reads go through one cached 256 KiB
//! block (scrolling reads neighbouring lines). Backends without range reads
//! keep one open stream and skip forward, reopening only to go back.
use std::io::{self, Read};
use std::sync::{Mutex, PoisonError};

use crate::textview::TextSource;
use crate::vfs::BackendHandle;

type ReadStream = Box<dyn Read + Send>;

const BLOCK: u64 = 256 * 1024;

/// A file of a backend as random-access bytes.
pub struct VfsSource {
    backend: BackendHandle,
    path: String,
    id: Option<String>,
    len: u64,
    ranged: bool,
    stream: Mutex<Option<(u64, ReadStream)>>,
    block: Mutex<Option<(u64, Vec<u8>)>>,
}

impl VfsSource {
    /// Looks up `path` (its length and backend id); folders, links to
    /// nothing and special files are refused.
    pub fn open(backend: BackendHandle, path: &str) -> io::Result<Self> {
        let meta = backend.stat(path)?;
        if meta.is_dir || meta.special {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{path} ist keine Datei"),
            ));
        }
        let ranged = backend.supports_range_reads(path);
        Ok(Self {
            backend,
            path: path.to_string(),
            id: meta.id,
            len: meta.size,
            ranged,
            stream: Mutex::new(None),
            block: Mutex::new(None),
        })
    }

    fn read_raw(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if self.ranged {
            if let Some(mut reader) = self.backend.open_read_range(
                &self.path,
                self.id.as_deref(),
                offset,
                buf.len() as u64,
            )? {
                return read_fully(&mut reader, buf);
            }
        }
        let mut guard = self.stream.lock().unwrap_or_else(PoisonError::into_inner);
        let reusable = matches!(guard.as_ref(), Some((pos, _)) if *pos <= offset);
        if !reusable {
            *guard = Some(self.open_stream(offset)?);
        }
        let Some((pos, reader)) = guard.as_mut() else {
            return Ok(0);
        };
        if *pos < offset {
            let skip = offset - *pos;
            let skipped = io::copy(&mut reader.by_ref().take(skip), &mut io::sink())?;
            *pos += skipped;
            if skipped < skip {
                return Ok(0);
            }
        }
        let n = read_fully(reader, buf)?;
        *pos += n as u64;
        Ok(n)
    }

    fn open_stream(&self, offset: u64) -> io::Result<(u64, ReadStream)> {
        if let Some(reader) = self
            .backend
            .open_read_at(&self.path, self.id.as_deref(), offset)?
        {
            return Ok((offset, reader));
        }
        Ok((0, self.backend.open_read(&self.path)?))
    }
}

impl TextSource for VfsSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        if buf.len() as u64 >= BLOCK {
            return self.read_raw(offset, buf);
        }
        let start = offset / BLOCK * BLOCK;
        let mut cached = self.block.lock().unwrap_or_else(PoisonError::into_inner);
        if cached.as_ref().map(|(s, _)| *s) != Some(start) {
            let size = BLOCK.min(self.len - start) as usize;
            let mut block = vec![0u8; size];
            let n = self.read_raw(start, &mut block)?;
            block.truncate(n);
            *cached = Some((start, block));
        }
        let Some((_, block)) = cached.as_ref() else {
            return Ok(0);
        };
        let from = (offset - start) as usize;
        if from >= block.len() {
            return Ok(0);
        }
        let n = buf.len().min(block.len() - from);
        buf[..n].copy_from_slice(&block[from..from + n]);
        Ok(n)
    }
}

fn read_fully(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}
