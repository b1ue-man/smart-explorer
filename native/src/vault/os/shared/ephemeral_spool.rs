//! The spool of a copy between two backends that cannot overlap a reader
//! and a writer (`vfs::copy_between`, K08 of the vault plan): when a vault
//! takes part, the bytes reach the anonymous temporary file only encrypted,
//! in the vault content format (header plus AES-GCM chunks) under a random
//! key that exists only in memory for this one copy. Dropping the spool
//! wipes the key; the file has no name and goes with its handle.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use crate::cryptomator::{
    CipherCombo, ContentCryptor, DecryptingReader, EncryptingWriter, Masterkey, MasterkeyCell,
};

/// An encrypted anonymous spool file with a key used once.
pub(crate) struct EphemeralSpool {
    file: File,
    cryptor: ContentCryptor,
}

#[cfg(test)]
thread_local! {
    /// Spools made on this thread (tests see which copies were sealed).
    pub(crate) static SEALED_SPOOLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl EphemeralSpool {
    pub(crate) fn new() -> io::Result<Self> {
        Self::in_file(tempfile::tempfile()?)
    }

    fn in_file(file: File) -> io::Result<Self> {
        let key = Masterkey::generate().map_err(io::Error::from)?;
        #[cfg(test)]
        SEALED_SPOOLS.with(|count| count.set(count.get() + 1));
        Ok(Self {
            file,
            cryptor: ContentCryptor::new(MasterkeyCell::new(key), CipherCombo::SivGcm),
        })
    }

    /// Fills the spool from `reader`: the cleartext bytes taken.
    pub(crate) fn fill(&mut self, reader: &mut dyn Read) -> io::Result<u64> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        let mut writer = EncryptingWriter::new(&mut self.file, self.cryptor.clone())?;
        let copied = io::copy(reader, &mut writer)?;
        writer.finish()?;
        Ok(copied)
    }

    /// The cleartext from the start; every chunk is authenticated.
    pub(crate) fn reader(&mut self) -> io::Result<impl Read + '_> {
        self.file.seek(SeekFrom::Start(0))?;
        DecryptingReader::new(&mut self.file, self.cryptor.clone())
    }

    /// The stored bytes as they lie in the file (tests).
    #[cfg(test)]
    pub(crate) fn stored(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "ephemeral_spool_tests.rs"]
mod tests;
