//! Streams of the vault backend: decrypting readers (from the start or from
//! any offset across chunk boundaries) and the encrypting writer. A writer
//! writes a new header and fresh chunk nonces into a stage of the storage;
//! `flush` closes that stage and, for `open_write`, publishes it. Dropping a
//! writer without `flush` publishes nothing and commits no storage stage.
//! Once the vault is locked, every stream fails with `VaultLocked`.
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use zeroize::{Zeroize, Zeroizing};

use crate::cryptomator::{
    ContentCryptor, DecryptingReader, EncryptingWriter, FileHeader, FormatError, CHUNK_PAYLOAD,
};
use crate::vault::core::VaultSession;
use crate::vfs::{self, Backend};

use super::backend::VaultBackend;
use super::backend_nodes::{invalid_data, invalid_input, NodeKind, Parts};

/// Publication of a written stage (`open_write`), run by `flush`.
pub(super) type Commit = Box<dyn FnOnce() -> io::Result<()> + Send>;
/// Removal of an internal stage nobody will publish.
pub(super) type Abandon = Box<dyn FnOnce() + Send>;

/// A reader whose errors become `VaultLocked` once the vault is locked;
/// it stops at once, also with decrypted bytes still buffered.
struct Guarded<R> {
    inner: R,
    session: Arc<VaultSession>,
}

impl<R: Read> Read for Guarded<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.session.check()?;
        self.inner
            .read(buffer)
            .map_err(|error| self.session.relock(error))
    }
}

fn read_full(reader: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Decrypts from chunk `index` on (the stored stream starts there) and
/// skips `skip` cleartext bytes of that first chunk.
struct OffsetReader {
    inner: Box<dyn Read + Send>,
    cryptor: ContentCryptor,
    header: FileHeader,
    index: u64,
    skip: usize,
    chunk: Vec<u8>,
    plain: Zeroizing<Vec<u8>>,
    position: usize,
    ended: bool,
    failed: Option<io::ErrorKind>,
}

impl OffsetReader {
    fn next_chunk(&mut self) -> io::Result<bool> {
        let read = read_full(&mut self.inner, &mut self.chunk)?;
        if read == 0 {
            return Ok(false);
        }
        let plain = self
            .cryptor
            .decrypt_chunk(&self.header, self.index, &self.chunk[..read])?;
        self.index += 1;
        self.ended = read < self.chunk.len();
        self.plain = plain;
        self.position = std::mem::take(&mut self.skip).min(self.plain.len());
        Ok(true)
    }
}

impl Read for OffsetReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(kind) = self.failed {
            return Err(io::Error::new(kind, "Tresordatei: früherer Fehler"));
        }
        while self.position >= self.plain.len() {
            if self.ended {
                return Ok(0);
            }
            match self.next_chunk() {
                Ok(true) => {}
                Ok(false) => {
                    self.ended = true;
                    return Ok(0);
                }
                Err(error) => {
                    self.failed = Some(error.kind());
                    self.plain.zeroize();
                    return Err(error);
                }
            }
        }
        let count = buffer.len().min(self.plain.len() - self.position);
        buffer[..count].copy_from_slice(&self.plain[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

impl VaultBackend {
    /// The stored content of the file at `parts`: an existing stage of that
    /// name, else the entry (folders and links are refused, links are
    /// never followed).
    pub(super) fn file_contents(&self, parts: &Parts<'_>) -> io::Result<String> {
        if let Some(rel) = self.stage_rel(parts)? {
            match self.inner.stat(&self.at(&rel)) {
                Ok(meta) if !meta.is_dir && !meta.is_symlink && !meta.special => return Ok(rel),
                Ok(_) => return Err(invalid_data("Stufe im Tresor ist keine Datei")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let found = self.find_at(parts)?;
        match found.kind {
            NodeKind::File { .. } => Ok(found.node.contents()),
            NodeKind::Dir { .. } => Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "Eintrag im Tresor ist ein Ordner",
            )),
            NodeKind::Symlink { .. } => Err(invalid_input(
                "Verknüpfungen im Tresor werden nicht verfolgt",
            )),
        }
    }

    /// `open_read`, `open_read_id` and (`regular`) `open_read_regular`.
    pub(super) fn reader(
        &self,
        path: &str,
        id: Option<&str>,
        regular: bool,
    ) -> io::Result<Box<dyn Read + Send>> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let parts = Parts::of(path)?
                .ok_or_else(|| invalid_input("Die Tresorwurzel ist keine Datei"))?;
            let at = self.at(&self.file_contents(&parts)?);
            let checked_id = self.checked_storage_id(&at, id)?;
            let id = checked_id.as_deref();
            let stored = if regular {
                vfs::open_read_regular(&*self.inner, &at, id)?
            } else {
                self.inner.open_read_id(&at, id)?
            };
            let reader = DecryptingReader::new(stored, self.session.cryptor())?;
            Ok(Box::new(Guarded {
                inner: reader,
                session: Arc::clone(&self.session),
            }) as Box<dyn Read + Send>)
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// `open_read_at`: the header, then the stored stream from the chunk
    /// that holds `offset`; `None` when the storage cannot start mid-file.
    pub(super) fn reader_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> io::Result<Option<Box<dyn Read + Send>>> {
        if offset == 0 {
            return self.reader(path, id, false).map(Some);
        }
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let parts = Parts::of(path)?
                .ok_or_else(|| invalid_input("Die Tresorwurzel ist keine Datei"))?;
            let at = self.at(&self.file_contents(&parts)?);
            let checked_id = self.checked_storage_id(&at, id)?;
            let id = checked_id.as_deref();
            let combo = self.session.config().combo;
            let index = offset / CHUNK_PAYLOAD as u64;
            let start = (combo.chunk_size() as u64)
                .checked_mul(index)
                .and_then(|body| body.checked_add(combo.header_size() as u64))
                .ok_or_else(|| invalid_input("Leseposition außerhalb der Datei"))?;
            let Some(stored) = self.inner.open_read_at(&at, id, start)? else {
                return Ok(None);
            };
            let mut head = self.inner.open_read_id(&at, id)?;
            let mut bytes = vec![0u8; combo.header_size()];
            if read_full(&mut head, &mut bytes)? != bytes.len() {
                return Err(FormatError::InvalidHeader.into());
            }
            drop(head);
            let cryptor = self.session.cryptor();
            let header = cryptor.decrypt_header(&bytes)?;
            let reader = OffsetReader {
                inner: stored,
                cryptor,
                header,
                index,
                skip: (offset % CHUNK_PAYLOAD as u64) as usize,
                chunk: vec![0u8; combo.chunk_size()],
                plain: Zeroizing::new(Vec::new()),
                position: 0,
                ended: false,
                failed: None,
            };
            Ok(Some(Box::new(Guarded {
                inner: reader,
                session: Arc::clone(&self.session),
            }) as Box<dyn Read + Send>))
        })();
        result.map_err(|error| self.session.relock(error))
    }
}

/// Passes writes through; `flush` reaches the storage only while open, so
/// the best-effort finish of a dropped `EncryptingWriter` never commits the
/// storage stage (a remote writer uploads on `flush`).
struct Gate {
    inner: Box<dyn Write + Send>,
    open: Arc<AtomicBool>,
}

impl Write for Gate {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.inner.write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.open.load(Ordering::SeqCst) {
            self.inner.flush()
        } else {
            Ok(())
        }
    }
}

enum State {
    Open,
    Done,
    Failed(io::ErrorKind),
}

/// Encrypts into a storage stage; see the module comment.
pub(super) struct VaultWriter {
    session: Arc<VaultSession>,
    writer: Option<EncryptingWriter<Gate>>,
    gate: Arc<AtomicBool>,
    /// Cleartext length a sized stage must receive exactly.
    expected: Option<u64>,
    written: u64,
    state: State,
    commit: Option<Commit>,
    abandon: Option<Abandon>,
}

impl VaultWriter {
    pub(super) fn open(
        session: &Arc<VaultSession>,
        stored: Box<dyn Write + Send>,
        expected: Option<u64>,
        commit: Option<Commit>,
        abandon: Option<Abandon>,
    ) -> io::Result<Self> {
        let gate = Arc::new(AtomicBool::new(false));
        let sink = Gate {
            inner: stored,
            open: Arc::clone(&gate),
        };
        let writer = match EncryptingWriter::new(sink, session.cryptor()) {
            Ok(writer) => writer,
            Err(error) => {
                if let Some(abandon) = abandon {
                    abandon();
                }
                return Err(session.relock(error));
            }
        };
        Ok(Self {
            session: Arc::clone(session),
            writer: Some(writer),
            gate,
            expected,
            written: 0,
            state: State::Open,
            commit,
            abandon,
        })
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        let error = self.session.relock(error);
        self.state = State::Failed(error.kind());
        self.gate.store(false, Ordering::SeqCst);
        self.writer = None;
        if let Some(abandon) = self.abandon.take() {
            abandon();
        }
        error
    }

    fn usable(&self) -> io::Result<()> {
        match self.state {
            State::Open => Ok(()),
            State::Done => Err(io::Error::other("Tresordatei ist bereits geschrieben")),
            State::Failed(kind) => Err(io::Error::new(kind, "Tresordatei: früherer Fehler")),
        }
    }

    /// Last chunk, storage flush (commits the stage), close, publish.
    fn complete(&mut self) -> io::Result<()> {
        self.session.check()?;
        if self
            .expected
            .is_some_and(|expected| expected != self.written)
        {
            return Err(invalid_data("Weniger Daten als angekündigt"));
        }
        let writer = self
            .writer
            .take()
            .ok_or_else(|| io::Error::other("Tresordatei ist bereits geschlossen"))?;
        self.gate.store(true, Ordering::SeqCst);
        let stored = writer.finish()?;
        drop(stored);
        // Locked meanwhile: the complete stage is never published.
        self.session.check()?;
        if let Some(commit) = self.commit.take() {
            commit()?;
        }
        self.abandon = None;
        Ok(())
    }
}

impl Write for VaultWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.usable()?;
        if let Err(error) = self.session.check() {
            return Err(self.fail(error));
        }
        let room = self
            .expected
            .map(|expected| expected.saturating_sub(self.written));
        if room.is_some_and(|room| (data.len() as u64) > room) {
            return Err(self.fail(invalid_data("Mehr Daten als angekündigt")));
        }
        let Some(writer) = self.writer.as_mut() else {
            return Err(io::Error::other("Tresordatei ist bereits geschlossen"));
        };
        match writer.write(data) {
            Ok(count) => {
                self.written += count as u64;
                Ok(count)
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Publishes (or only closes the stage); later calls return `Ok`.
    fn flush(&mut self) -> io::Result<()> {
        if matches!(self.state, State::Done) {
            return Ok(());
        }
        self.usable()?;
        match self.complete() {
            Ok(()) => {
                self.state = State::Done;
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }
}

impl Drop for VaultWriter {
    fn drop(&mut self) {
        if matches!(self.state, State::Done) {
            return;
        }
        self.gate.store(false, Ordering::SeqCst);
        self.writer = None;
        if let Some(abandon) = self.abandon.take() {
            abandon();
        }
    }
}
