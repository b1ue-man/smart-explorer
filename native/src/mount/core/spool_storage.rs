//! Spool content storage of the mount engine (contract V1, S2-V2).
//!
//! The engine keeps one whole-file spool per open or cached remote file in
//! `<spool-root>/<mount-id>/files/<32 hex>.spool` and reaches its content
//! only through `SpoolStorage`. The plain storage keeps the layout of every
//! earlier release: the file holds exactly the file content. A vault drive
//! passes S3's sealed storage (`SealedSpoolFile`, Cryptomator content
//! format), so no plain text of a vault reaches the disk. Name and link
//! checks stay in `spool.rs` for both.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use super::path_sealer::{PathSealer, PlainPathSealer};

/// One open spool file; offsets and lengths address the plain content.
pub trait SpoolFile: Send {
    /// Reads at `offset`; fewer bytes than requested only at the end of the
    /// content or after an interruption, 0 at or after the end.
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<usize>;
    /// Writes all of `input` at `offset`, extending the content when needed.
    fn write_at(&mut self, offset: u64, input: &[u8]) -> io::Result<()>;
    /// Truncates the content, or extends it with zero bytes, to `length`.
    fn set_len(&mut self, length: u64) -> io::Result<()>;
    /// Length of the content.
    fn len(&self) -> io::Result<u64>;
    fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }
    /// Makes the written content durable.
    fn sync_data(&mut self) -> io::Result<()>;
}

/// Opens spool files whose path `spool.rs` has already validated.
pub trait SpoolStorage: Send + Sync {
    /// Creates `path` exclusively; `AlreadyExists` when the name is taken.
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn SpoolFile>>;
    /// Opens an existing spool file.
    fn open(&self, path: &Path, writable: bool) -> io::Result<Box<dyn SpoolFile>>;
}

/// Spool and path protection of one drive (contract V1).
#[derive(Clone)]
pub struct MountStorage {
    pub spool: Arc<dyn SpoolStorage>,
    pub paths: Arc<dyn PathSealer>,
}

impl MountStorage {
    /// Every drive without a vault: plain spool bytes and plain paths, the
    /// bytes of every earlier release.
    pub fn plain() -> Self {
        Self {
            spool: Arc::new(PlainSpoolStorage),
            paths: Arc::new(PlainPathSealer),
        }
    }
}

/// Plain content through one OS file handle per open.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlainSpoolStorage;

impl SpoolStorage for PlainSpoolStorage {
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn SpoolFile>> {
        OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .map(PlainSpoolFile::boxed)
    }

    fn open(&self, path: &Path, writable: bool) -> io::Result<Box<dyn SpoolFile>> {
        OpenOptions::new()
            .read(true)
            .write(writable)
            .open(path)
            .map(PlainSpoolFile::boxed)
    }
}

/// The remembered OS position saves the seek of every sequential call, so a
/// sequential download or upload issues the same system calls as the plain
/// `File` the engine used before V1.
struct PlainSpoolFile {
    file: File,
    position: Option<u64>,
}

impl PlainSpoolFile {
    fn boxed(file: File) -> Box<dyn SpoolFile> {
        Box::new(Self {
            file,
            position: Some(0),
        })
    }

    fn seek_to(&mut self, offset: u64) -> io::Result<()> {
        if self.position != Some(offset) {
            self.position = None;
            self.file.seek(SeekFrom::Start(offset))?;
            self.position = Some(offset);
        }
        Ok(())
    }
}

impl SpoolFile for PlainSpoolFile {
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.seek_to(offset)?;
        match self.file.read(output) {
            Ok(read) => {
                self.position = Some(offset.saturating_add(read as u64));
                Ok(read)
            }
            Err(error) => {
                self.position = None;
                Err(error)
            }
        }
    }

    fn write_at(&mut self, offset: u64, input: &[u8]) -> io::Result<()> {
        self.seek_to(offset)?;
        match self.file.write_all(input) {
            Ok(()) => {
                self.position = Some(offset.saturating_add(input.len() as u64));
                Ok(())
            }
            Err(error) => {
                self.position = None;
                Err(error)
            }
        }
    }

    fn set_len(&mut self, length: u64) -> io::Result<()> {
        self.file.set_len(length)
    }

    fn len(&self) -> io::Result<u64> {
        self.file.metadata().map(|metadata| metadata.len())
    }

    fn sync_data(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }
}

/// A spool file with a position. It stands where the engine's whole-file
/// code used `std::fs::File` before V1 (`Read`, `Write`, `Seek`,
/// `metadata().len()`, `set_len`, `sync_data`), so that code reads and writes
/// every spool through `SpoolStorage` without knowing the storage.
pub(crate) struct SpoolHandle {
    file: Box<dyn SpoolFile>,
    position: u64,
}

/// The part of the file metadata the engine reads: the content length.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpoolMetadata {
    len: u64,
}

impl SpoolMetadata {
    pub(crate) fn len(&self) -> u64 {
        self.len
    }
}

impl SpoolHandle {
    pub(crate) fn new(file: Box<dyn SpoolFile>) -> Self {
        Self { file, position: 0 }
    }

    pub(crate) fn metadata(&self) -> io::Result<SpoolMetadata> {
        self.file.len().map(|len| SpoolMetadata { len })
    }

    pub(crate) fn set_len(&mut self, length: u64) -> io::Result<()> {
        self.file.set_len(length)
    }

    pub(crate) fn sync_data(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }
}

impl Read for SpoolHandle {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read_at(self.position, output)?;
        self.position = self.position.saturating_add(read as u64);
        Ok(read)
    }
}

impl Write for SpoolHandle {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.file.write_at(self.position, input)?;
        self.position = self.position.saturating_add(input.len() as u64);
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for SpoolHandle {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let position = match target {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.file.len()?.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        }
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "spool seek before the start or beyond the addressable range",
            )
        })?;
        self.position = position;
        Ok(position)
    }
}
