//! Reading one archive entry without decompressing it into memory as a
//! whole (plan K7). The archive is parsed once per backend; every reader
//! clones that parse (an `Arc` of the central directory) with its own lazily
//! opened file handle. zip's entry reader borrows its archive, so a larger
//! entry is decompressed by a thread of its own that hands over bounded
//! chunks; an entry that fits in one chunk is decompressed right away into a
//! reserved buffer, which costs no more memory and no thread.
use crate::transfer::{try_reserve_memory, MemoryReservation};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use zip::result::ZipError;
use zip::ZipArchive;

/// Bytes a streamed entry hands over at once. A streaming reader holds at
/// most three (one being filled, one queued, one being read): 768 KiB, below
/// the 1 MiB an operation buffers anyway (recherche §2).
pub(super) const STREAM_CHUNK: usize = 256 * 1024;

/// The archive file of one reader, opened on first use: cloning the parse
/// for a new reader costs no handle until it reads, and the backend itself
/// keeps none open (the file stays movable and deletable on Windows).
pub(super) struct ArchiveFile {
    path: Arc<PathBuf>,
    file: Option<std::fs::File>,
    position: u64,
}

impl ArchiveFile {
    /// With the handle the caller already opened (so a missing archive
    /// fails the open with its own error kind).
    pub(super) fn opened(path: PathBuf, file: std::fs::File) -> Self {
        Self {
            path: Arc::new(path),
            file: Some(file),
            position: 0,
        }
    }

    fn handle(&mut self) -> io::Result<&mut std::fs::File> {
        if self.file.is_none() {
            let mut file = std::fs::File::open(self.path.as_ref())?;
            file.seek(SeekFrom::Start(self.position))?;
            self.file = Some(file);
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("ZIP-Datei ist nicht geöffnet"))
    }
}

impl Clone for ArchiveFile {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            file: None,
            position: self.position,
        }
    }
}

impl Read for ArchiveFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.handle()?.read(buffer)?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for ArchiveFile {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let position = self.handle()?.seek(target)?;
        self.position = position;
        Ok(position)
    }
}

pub(super) type Archive = ZipArchive<ArchiveFile>;

/// A file-system failure keeps its own kind (a vanished archive stays
/// `NotFound`); archive errors carry their text.
fn zip_err(error: ZipError) -> io::Error {
    match error {
        ZipError::Io(error) => error,
        error => io::Error::other(error.to_string()),
    }
}

/// A small entry, decompressed at open; its buffer is reserved.
struct Inline {
    data: Cursor<Vec<u8>>,
    _reservation: MemoryReservation,
}

impl Read for Inline {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.data.read(buffer)
    }
}

enum Piece {
    Data(Vec<u8>),
    End,
    Failed(io::Error),
}

/// A larger entry, decompressed by its own thread one chunk ahead.
pub(super) struct Streamed {
    pieces: crossbeam_channel::Receiver<Piece>,
    current: Vec<u8>,
    consumed: usize,
    ended: bool,
    #[cfg(test)]
    pub(super) produced: Arc<std::sync::atomic::AtomicUsize>,
}

impl Read for Streamed {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || (self.ended && self.consumed >= self.current.len()) {
            return Ok(0);
        }
        if self.consumed >= self.current.len() {
            match self.pieces.recv() {
                Ok(Piece::Data(data)) => {
                    self.current = data;
                    self.consumed = 0;
                }
                Ok(Piece::End) => {
                    self.ended = true;
                    return Ok(0);
                }
                Ok(Piece::Failed(error)) => return Err(error),
                // The thread ended without saying so: never a short success.
                Err(_) => return Err(io::Error::other("ZIP-Eintrag endete unerwartet")),
            }
        }
        let available = &self.current[self.consumed..];
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.consumed += count;
        Ok(count)
    }
}

/// Fills `chunk` as far as the entry goes; fewer bytes only at its end.
fn fill(entry: &mut impl Read, chunk: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < chunk.len() {
        match entry.read(&mut chunk[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Decompresses entry `index` on a thread of its own; dropping the reader
/// stops the thread at its next chunk.
pub(super) fn stream(mut archive: Archive, index: usize) -> io::Result<Streamed> {
    let (sender, pieces) = crossbeam_channel::bounded::<Piece>(1);
    #[cfg(test)]
    let produced = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    #[cfg(test)]
    let counter = produced.clone();
    std::thread::Builder::new()
        .name("zip-read".to_string())
        .spawn(move || {
            let mut entry = match archive.by_index(index) {
                Ok(entry) => entry,
                Err(error) => {
                    let _ = sender.send(Piece::Failed(zip_err(error)));
                    return;
                }
            };
            loop {
                let mut chunk = vec![0u8; STREAM_CHUNK];
                let piece = match fill(&mut entry, &mut chunk) {
                    Ok(0) => Piece::End,
                    Ok(filled) => {
                        chunk.truncate(filled);
                        Piece::Data(chunk)
                    }
                    Err(error) => Piece::Failed(error),
                };
                #[cfg(test)]
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let last = !matches!(piece, Piece::Data(_));
                if sender.send(piece).is_err() || last {
                    return;
                }
            }
        })?;
    Ok(Streamed {
        pieces,
        current: Vec::new(),
        consumed: 0,
        ended: false,
        #[cfg(test)]
        produced,
    })
}

/// A reader of entry `index` whose uncompressed size is `size`.
pub(super) fn open(archive: &Archive, index: usize, size: u64) -> io::Result<Box<dyn Read + Send>> {
    if size <= STREAM_CHUNK as u64 {
        if let Some(reservation) = try_reserve_memory(size) {
            let mut archive = archive.clone();
            let mut entry = archive.by_index(index).map_err(zip_err)?;
            let mut data = Vec::with_capacity(STREAM_CHUNK.min(size as usize));
            // One byte more than announced shows a lying directory entry;
            // reaching the end lets zip check the CRC.
            entry.by_ref().take(size + 1).read_to_end(&mut data)?;
            if data.len() as u64 != size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "ZIP-Eintrag ist größer als im Verzeichnis angegeben",
                ));
            }
            return Ok(Box::new(Inline {
                data: Cursor::new(data),
                _reservation: reservation,
            }));
        }
    }
    Ok(Box::new(stream(archive.clone(), index)?))
}
