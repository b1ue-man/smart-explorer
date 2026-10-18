//! Sealed spool content of vault drives (contract V1, S2-V2, C3.6, VLT-M12).
//!
//! A vault drive keeps its whole-file spools where every drive keeps them
//! (`<spool-root>/<mount-id>/files/<32 hex>.spool`), but each file holds the
//! vault content format `SIV_GCM` under the content key of the drive's spool
//! key: a 68-byte header (nonce ‖ encrypted file key ‖ tag), then chunks of
//! at most 32 KiB cleartext, each nonce ‖ AES-256-GCM ciphertext ‖ tag with
//! the chunk number and the header nonce as associated data. No cleartext
//! byte of the vault reaches the disk.
//!
//! Random access: a read decrypts the chunks it touches; a write reads the
//! chunks it changes only in part, overlays the new bytes and encrypts each
//! changed chunk again with a fresh random nonce, so no nonce repeats under
//! one file key. A chunk in the middle of a file is always full (CryptoFS
//! rule); growing a file fills the gap with zero bytes. Every call reaches
//! the file at once, like the plain storage, and the length comes from the
//! file size on every call, so all open handles of one spool see each
//! other's writes.
//!
//! Limits of the format: a chunk moved to another number or another file
//! fails its tag; cutting a file at a chunk boundary is not detected (the
//! format carries no length tag). A crash in the middle of rewriting a
//! chunk leaves a chunk that fails its tag instead of mixed bytes.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use zeroize::Zeroizing;

use crate::cryptomator::{
    ciphertext_size, cleartext_file_size, CipherCombo, ContentCryptor, FileHeader, CHUNK_PAYLOAD,
};
use crate::mount::{SpoolFile, SpoolStorage};

use super::spool_key::SpoolKey;

const COMBO: CipherCombo = CipherCombo::SivGcm;
const PAYLOAD: u64 = CHUNK_PAYLOAD as u64;

/// Opens the spool files of one vault drive under its content key.
pub(crate) struct SealedSpoolStorage {
    cryptor: ContentCryptor,
}

impl SealedSpoolStorage {
    pub(crate) fn new(key: &SpoolKey) -> io::Result<Self> {
        Ok(Self {
            cryptor: key.content_cryptor()?,
        })
    }
}

impl SpoolStorage for SealedSpoolStorage {
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn SpoolFile>> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)?;
        let mut sealed = SealedSpoolFile::new(file, self.cryptor.clone());
        sealed.write_new_header()?;
        Ok(Box::new(sealed))
    }

    fn open(&self, path: &Path, writable: bool) -> io::Result<Box<dyn SpoolFile>> {
        let file = OpenOptions::new().read(true).write(writable).open(path)?;
        let mut sealed = SealedSpoolFile::new(file, self.cryptor.clone());
        // A file whose header never reached the disk has no content; its
        // header is written with the first write.
        if sealed.file_size()? > 0 {
            sealed.load_header()?;
        }
        Ok(Box::new(sealed))
    }
}

/// One open spool file; offsets and lengths address the cleartext.
pub(crate) struct SealedSpoolFile {
    file: File,
    cryptor: ContentCryptor,
    header: Option<FileHeader>,
}

impl SealedSpoolFile {
    fn new(file: File, cryptor: ContentCryptor) -> Self {
        Self {
            file,
            cryptor,
            header: None,
        }
    }

    fn file_size(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    /// Cleartext length from the file size; an empty file has no header.
    fn plain_len(&self) -> io::Result<u64> {
        match self.file_size()? {
            0 => Ok(0),
            size => cleartext_file_size(size, COMBO).map_err(io::Error::from),
        }
    }

    fn write_new_header(&mut self) -> io::Result<()> {
        let header = self.cryptor.new_header()?;
        let bytes = self.cryptor.encrypt_header(&header)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&bytes)?;
        self.header = Some(header);
        Ok(())
    }

    fn load_header(&mut self) -> io::Result<()> {
        if self.header.is_some() {
            return Ok(());
        }
        let mut bytes = vec![0u8; COMBO.header_size()];
        self.file.seek(SeekFrom::Start(0))?;
        self.file
            .read_exact(&mut bytes)
            .map_err(|_| invalid("Kopf der versiegelten Spool-Datei unvollständig"))?;
        self.header = Some(self.cryptor.decrypt_header(&bytes)?);
        Ok(())
    }

    fn header(&self) -> io::Result<&FileHeader> {
        self.header
            .as_ref()
            .ok_or_else(|| invalid("Kopf der versiegelten Spool-Datei fehlt"))
    }

    /// The cleartext of chunk `index` of content `len` bytes long; empty
    /// beyond the end.
    fn read_chunk(&mut self, index: u64, len: u64) -> io::Result<Zeroizing<Vec<u8>>> {
        let start = index * PAYLOAD;
        if start >= len {
            return Ok(Zeroizing::new(Vec::new()));
        }
        let plain = (len - start).min(PAYLOAD) as usize;
        let mut sealed = vec![0u8; plain + COMBO.chunk_overhead()];
        self.file.seek(SeekFrom::Start(chunk_offset(index)))?;
        self.file.read_exact(&mut sealed)?;
        let header = self.header()?;
        self.cryptor
            .decrypt_chunk(header, index, &sealed)
            .map_err(io::Error::from)
    }

    /// Encrypts `plain` as chunk `index` with a fresh nonce and writes it.
    fn write_chunk(&mut self, index: u64, plain: &[u8]) -> io::Result<()> {
        let sealed = {
            let header = self.header()?;
            self.cryptor.encrypt_chunk(header, index, plain)?
        };
        self.file.seek(SeekFrom::Start(chunk_offset(index)))?;
        self.file.write_all(&sealed)
    }

    /// Appends zero bytes from `from` to `to`, one chunk step at a time.
    fn grow(&mut self, from: u64, to: u64) -> io::Result<()> {
        let zeros = vec![0u8; CHUNK_PAYLOAD];
        let mut at = from;
        while at < to {
            let step = (to - at).min(PAYLOAD - at % PAYLOAD);
            self.write_at(at, &zeros[..step as usize])?;
            at += step;
        }
        Ok(())
    }
}

/// Where chunk `index` starts in the file.
fn chunk_offset(index: u64) -> u64 {
    COMBO.header_size() as u64 + index * COMBO.chunk_size() as u64
}

impl SpoolFile for SealedSpoolFile {
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        let len = self.plain_len()?;
        if output.is_empty() || offset >= len {
            return Ok(0);
        }
        self.load_header()?;
        let mut done = 0usize;
        while done < output.len() {
            let position = offset + done as u64;
            if position >= len {
                break;
            }
            let index = position / PAYLOAD;
            let within = (position % PAYLOAD) as usize;
            let plain = self.read_chunk(index, len)?;
            let take = (plain.len() - within).min(output.len() - done);
            output[done..done + take].copy_from_slice(&plain[within..within + take]);
            done += take;
        }
        Ok(done)
    }

    fn write_at(&mut self, offset: u64, input: &[u8]) -> io::Result<()> {
        if input.is_empty() {
            return Ok(());
        }
        let end = offset
            .checked_add(input.len() as u64)
            .ok_or_else(|| invalid("Spool-Versatz außerhalb des Bereichs"))?;
        if self.file_size()? == 0 {
            self.write_new_header()?;
        } else {
            self.load_header()?;
        }
        let len = self.plain_len()?;
        let new_len = len.max(end);
        let first = offset.min(len) / PAYLOAD;
        let last = (end - 1) / PAYLOAD;
        for index in first..=last {
            let start = index * PAYLOAD;
            let chunk_len = (new_len - start).min(PAYLOAD) as usize;
            let chunk_end = start + chunk_len as u64;
            // Old bytes only where the input does not cover the chunk; a
            // chunk in a gap stays zero.
            let mut plain = Zeroizing::new(vec![0u8; chunk_len]);
            if offset > start || end < chunk_end {
                let old = self.read_chunk(index, len)?;
                let keep = old.len().min(chunk_len);
                plain[..keep].copy_from_slice(&old[..keep]);
            }
            let from = offset.max(start);
            let to = end.min(chunk_end);
            if from < to {
                plain[(from - start) as usize..(to - start) as usize]
                    .copy_from_slice(&input[(from - offset) as usize..(to - offset) as usize]);
            }
            self.write_chunk(index, &plain)?;
        }
        Ok(())
    }

    fn set_len(&mut self, length: u64) -> io::Result<()> {
        let len = self.plain_len()?;
        if length > len {
            return self.grow(len, length);
        }
        if length == len {
            return Ok(());
        }
        self.load_header()?;
        let within = (length % PAYLOAD) as usize;
        if within != 0 {
            // The new last chunk is shorter: encrypted again, fresh nonce.
            let index = length / PAYLOAD;
            let plain = self.read_chunk(index, len)?;
            self.write_chunk(index, &plain[..within])?;
        }
        self.file
            .set_len(COMBO.header_size() as u64 + ciphertext_size(length, COMBO))
    }

    fn len(&self) -> io::Result<u64> {
        self.plain_len()
    }

    fn sync_data(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
