//! Streams over encrypted file contents: header, then chunks of 32 KiB
//! cleartext each, numbered from 0. Readers authenticate every chunk before
//! they return any of its bytes; after a failure they return only errors.
//! Writers write the CryptoFS form: an empty file is only the header, and a
//! file never ends with an empty chunk (C1.7). Readers accept both empty
//! forms, header only and header plus one empty chunk (CryptoLib's stream
//! writer, W3).
//!
//! Both hold the vault's `MasterkeyCell`: once the vault is locked, the next
//! chunk fails with `FormatError::Locked` (`PermissionDenied`).
use std::io::{self, Read, Write};
use std::sync::Arc;

use zeroize::{Zeroize, Zeroizing};

use super::cipher_combo::{CipherCombo, CHUNK_PAYLOAD};
use super::content_batch::{decrypt_chunks, encrypt_chunks, BATCH_CHUNKS};
use super::content_chunks::{decrypt_chunk, encrypt_chunk};
use super::content_header::{decrypt_header, encrypt_header, FileHeader};
use super::keys::{Masterkey, MasterkeyCell};
use super::FormatError;

/// Encrypts and decrypts file contents of one unlocked vault.
#[derive(Clone, Debug)]
pub(crate) struct ContentCryptor {
    keys: Arc<MasterkeyCell>,
    combo: CipherCombo,
}

impl ContentCryptor {
    pub(crate) fn new(keys: Arc<MasterkeyCell>, combo: CipherCombo) -> Self {
        Self { keys, combo }
    }

    pub(crate) fn combo(&self) -> CipherCombo {
        self.combo
    }

    /// A random header for a new file.
    pub(crate) fn new_header(&self) -> Result<FileHeader, FormatError> {
        if self.keys.is_wiped() {
            return Err(FormatError::Locked);
        }
        FileHeader::generate(self.combo)
    }

    pub(crate) fn encrypt_header(&self, header: &FileHeader) -> Result<Vec<u8>, FormatError> {
        self.keys.with(|key| encrypt_header(key, header))?
    }

    /// Decrypts a header read from the start of a file (exact header size).
    pub(crate) fn decrypt_header(&self, bytes: &[u8]) -> Result<FileHeader, FormatError> {
        self.keys
            .with(|key| decrypt_header(key, self.combo, bytes))?
    }

    /// Encrypts chunk `index` (at most 32 KiB) with a fresh nonce.
    pub(crate) fn encrypt_chunk(
        &self,
        header: &FileHeader,
        index: u64,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, FormatError> {
        self.keys
            .with(|key| encrypt_chunk(key, header, index, plaintext))?
    }

    /// Authenticates and decrypts chunk `index` (random access: the chunk
    /// starts at `header_size + index · chunk_size`).
    pub(crate) fn decrypt_chunk(
        &self,
        header: &FileHeader,
        index: u64,
        chunk: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, FormatError> {
        self.keys
            .with(|key| decrypt_chunk(key, header, index, chunk))?
    }
}

impl ContentCryptor {
    /// The masterkey for one batch: the vault lock is checked once per
    /// batch, and the parallel chunk work never runs under the key's lock
    /// (a waiting `wipe` must not stall the workers that hold the key).
    /// The copy lives in zeroizing buffers and is wiped after the batch.
    fn batch_key(&self) -> Result<Masterkey, FormatError> {
        self.keys.with(|key| Masterkey::from_raw(&key.raw()))
    }

    /// Encrypts consecutive chunks from `first` in one batch (every chunk
    /// but the last one full).
    pub(crate) fn encrypt_chunks(
        &self,
        header: &FileHeader,
        first: u64,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, FormatError> {
        encrypt_chunks(&self.batch_key()?, header, first, plaintext)
    }

    /// Authenticates and decrypts consecutive chunks from `first`.
    pub(crate) fn decrypt_chunks(
        &self,
        header: &FileHeader,
        first: u64,
        ciphertext: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, FormatError> {
        decrypt_chunks(&self.batch_key()?, header, first, ciphertext)
    }
}

/// Reads until `buffer` is full or the source ends; returns the bytes read.
fn read_full(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
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

/// Decrypts an encrypted file read from `inner`.
pub(crate) struct DecryptingReader<R: Read> {
    inner: R,
    cryptor: ContentCryptor,
    header: FileHeader,
    index: u64,
    /// Chunks the next read asks for (1, 2, 4, … up to `BATCH_CHUNKS`).
    batch: usize,
    chunk: Vec<u8>,
    plain: Zeroizing<Vec<u8>>,
    position: usize,
    ended: bool,
    failed: Option<io::ErrorKind>,
}

impl<R: Read> DecryptingReader<R> {
    /// Reads and authenticates the header.
    pub(crate) fn new(mut inner: R, cryptor: ContentCryptor) -> io::Result<Self> {
        let mut header = vec![0u8; cryptor.combo().header_size()];
        if read_full(&mut inner, &mut header)? != header.len() {
            return Err(FormatError::InvalidHeader.into());
        }
        let header = cryptor.decrypt_header(&header)?;
        // One read and one parallel authentication per batch of chunks.
        let chunk = vec![0u8; cryptor.combo().chunk_size() * BATCH_CHUNKS];
        Ok(Self {
            inner,
            cryptor,
            header,
            index: 0,
            batch: 1,
            chunk,
            plain: Zeroizing::new(Vec::new()),
            position: 0,
            ended: false,
            failed: None,
        })
    }

    /// Decrypts the next batch of chunks into `plain`; `false` at the end
    /// of the file.
    fn next_chunk(&mut self) -> io::Result<bool> {
        // The batch grows from one chunk, so a short read (a file type
        // sniff) never pulls a whole batch over the network.
        let size = self.cryptor.combo().chunk_size();
        let wanted = (size * self.batch).min(self.chunk.len());
        let read = read_full(&mut self.inner, &mut self.chunk[..wanted])?;
        if read == 0 {
            return Ok(false);
        }
        let plain = self
            .cryptor
            .decrypt_chunks(&self.header, self.index, &self.chunk[..read])?;
        self.index += read.div_ceil(size) as u64;
        self.batch = (self.batch * 2).min(BATCH_CHUNKS);
        // Only a full batch can be followed by another chunk.
        self.ended = read < wanted;
        self.plain = plain;
        self.position = 0;
        Ok(true)
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.failed = Some(error.kind());
        self.plain.zeroize();
        self.position = 0;
        error
    }
}

impl<R: Read> Read for DecryptingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(kind) = self.failed {
            return Err(io::Error::new(
                kind,
                "verschlüsselte Datei: früherer Fehler",
            ));
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
                Err(error) => return Err(self.fail(error)),
            }
        }
        let count = buffer.len().min(self.plain.len() - self.position);
        buffer[..count].copy_from_slice(&self.plain[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

/// Encrypts into `inner`. `finish` writes the last chunk and must be
/// called; dropping an unfinished writer finishes it on a best-effort basis
/// (like `BufWriter`), unless a write already failed.
pub(crate) struct EncryptingWriter<W: Write> {
    inner: Option<W>,
    cryptor: ContentCryptor,
    header: FileHeader,
    header_written: bool,
    index: u64,
    pending: Zeroizing<Vec<u8>>,
    failed: bool,
}

impl<W: Write> EncryptingWriter<W> {
    /// A writer for a new file with a fresh random header.
    pub(crate) fn new(inner: W, cryptor: ContentCryptor) -> io::Result<Self> {
        let header = cryptor.new_header()?;
        Ok(Self {
            inner: Some(inner),
            cryptor,
            header,
            header_written: false,
            index: 0,
            pending: Zeroizing::new(Vec::with_capacity(CHUNK_PAYLOAD * BATCH_CHUNKS)),
            failed: false,
        })
    }

    fn sink(&mut self) -> io::Result<&mut W> {
        self.inner
            .as_mut()
            .ok_or_else(|| io::Error::other("verschlüsselte Datei bereits abgeschlossen"))
    }

    fn write_header(&mut self) -> io::Result<()> {
        if !self.header_written {
            let bytes = self.cryptor.encrypt_header(&self.header)?;
            self.sink()?.write_all(&bytes)?;
            self.header_written = true;
        }
        Ok(())
    }

    /// Encrypts all pending cleartext as the next chunks (the last one may
    /// be partial only when the file ends).
    fn emit(&mut self) -> io::Result<()> {
        self.emit_prefix(self.pending.len())
    }

    /// Encrypts the first `length` pending bytes (whole chunks, or all of
    /// them at the end) and writes them to the inner writer in one call.
    fn emit_prefix(&mut self, length: usize) -> io::Result<()> {
        if length == 0 {
            return Ok(());
        }
        let chunks =
            self.cryptor
                .encrypt_chunks(&self.header, self.index, &self.pending[..length])?;
        self.sink()?.write_all(&chunks)?;
        self.index += length.div_ceil(CHUNK_PAYLOAD) as u64;
        self.pending[..length].zeroize();
        self.pending.drain(..length);
        Ok(())
    }

    fn checked<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.failed = true;
            self.pending.zeroize();
        }
        result
    }

    fn finish_inner(&mut self) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("verschlüsselte Datei: früherer Fehler"));
        }
        let result = self.write_header().and_then(|()| {
            if self.pending.is_empty() {
                Ok(())
            } else {
                self.emit()
            }
        });
        let result = result.and_then(|()| self.sink()?.flush());
        self.checked(result)
    }

    /// Writes the header (empty file) or the last chunk, flushes and returns
    /// the inner writer.
    pub(crate) fn finish(mut self) -> io::Result<W> {
        self.finish_inner()?;
        self.inner
            .take()
            .ok_or_else(|| io::Error::other("verschlüsselte Datei bereits abgeschlossen"))
    }
}

impl<W: Write> Write for EncryptingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.failed {
            return Err(io::Error::other("verschlüsselte Datei: früherer Fehler"));
        }
        let header = self.write_header();
        self.checked(header)?;
        let room = CHUNK_PAYLOAD * BATCH_CHUNKS - self.pending.len();
        let count = room.min(buffer.len());
        self.pending.extend_from_slice(&buffer[..count]);
        if self.pending.len() == CHUNK_PAYLOAD * BATCH_CHUNKS {
            let emitted = self.emit();
            self.checked(emitted)?;
        }
        Ok(count)
    }

    /// Writes every full pending chunk and flushes the inner writer; a
    /// partial chunk stays pending (a chunk in the middle of a file is
    /// always full).
    fn flush(&mut self) -> io::Result<()> {
        let full = self.pending.len() / CHUNK_PAYLOAD * CHUNK_PAYLOAD;
        let emitted = self.emit_prefix(full);
        self.checked(emitted)?;
        let flushed = self.sink().and_then(|inner| inner.flush());
        self.checked(flushed)
    }
}

impl<W: Write> Drop for EncryptingWriter<W> {
    fn drop(&mut self) {
        if self.inner.is_some() && !self.failed && !std::thread::panicking() {
            let _ = self.finish_inner();
        }
        self.pending.zeroize();
    }
}

/// A whole small encrypted file in memory (`dirid.c9r`, `symlink.c9r`):
/// header, then the chunks; an empty `plaintext` gives only the header.
pub(crate) fn encrypt_bytes(
    cryptor: &ContentCryptor,
    plaintext: &[u8],
) -> Result<Vec<u8>, FormatError> {
    let header = cryptor.new_header()?;
    let mut out = cryptor.encrypt_header(&header)?;
    for (index, part) in plaintext.chunks(CHUNK_PAYLOAD).enumerate() {
        out.extend_from_slice(&cryptor.encrypt_chunk(&header, index as u64, part)?);
    }
    Ok(out)
}

/// The cleartext of a whole encrypted file in memory, refused above `limit`
/// cleartext bytes (`dirid.c9r`: 36 + 1, `symlink.c9r`: 4 · 32 767).
pub(crate) fn decrypt_bytes(
    cryptor: &ContentCryptor,
    ciphertext: &[u8],
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>, FormatError> {
    let combo = cryptor.combo();
    let body = ciphertext
        .get(combo.header_size()..)
        .ok_or(FormatError::InvalidHeader)?;
    let header = cryptor.decrypt_header(&ciphertext[..combo.header_size()])?;
    let mut out = Zeroizing::new(Vec::new());
    for (index, chunk) in body.chunks(combo.chunk_size()).enumerate() {
        let plain = cryptor.decrypt_chunk(&header, index as u64, chunk)?;
        if out.len() + plain.len() > limit {
            return Err(FormatError::TooLarge("Entschlüsselter Inhalt"));
        }
        out.extend_from_slice(&plain);
    }
    Ok(out)
}
