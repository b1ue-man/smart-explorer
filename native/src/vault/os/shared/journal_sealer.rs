//! Sealed journal fields of vault drives (contract V1, S2-V3, P1, VLT-M12).
//!
//! The drive host holds only the spool key of its drive, so it seals the
//! path fields of `journal.jsonl` and `stages.json` itself, with the text
//! key derived from the spool key (`se-mount-journal-v1`), without one IPC
//! call per journal record. Every field is the vault content format
//! `SIV_GCM` of its UTF-8 bytes as Base64url without padding; each seal has
//! a fresh header, so equal paths never give equal texts. The free texts of
//! conflicts (`detail`) may name paths and are sealed the same way (decision
//! on U02 open point 5). A field this key did not seal yields no text.
//!
//! `sealed_mount_storage` is the storage a vault drive's engine opens with:
//! sealed spool files and sealed journal fields under one spool key.
use std::io;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

use crate::cryptomator::{decrypt_bytes, encrypt_bytes, ContentCryptor};
use crate::mount::{MountStorage, PathSealer};

use super::sealed_spool::SealedSpoolStorage;
use super::spool_key::SpoolKey;

/// Largest cleartext of one sealed field; a longer conflict text is cut at
/// this bound (paths of every frontend stay far below it).
pub(crate) const MAX_FIELD: usize = 64 * 1024;

/// Seals the persisted path fields and conflict texts of one vault drive.
pub(crate) struct JournalPathSealer {
    cryptor: ContentCryptor,
}

impl JournalPathSealer {
    pub(crate) fn new(key: &SpoolKey) -> io::Result<Self> {
        Ok(Self {
            cryptor: key.text_cryptor()?,
        })
    }

    fn seal_text(&self, text: &str) -> io::Result<String> {
        if text.len() > MAX_FIELD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Journalfeld größer als 64 KiB",
            ));
        }
        let sealed = encrypt_bytes(&self.cryptor, text.as_bytes())?;
        Ok(URL_SAFE_NO_PAD.encode(sealed))
    }

    fn unseal_text(&self, sealed: &str) -> io::Result<String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(sealed.as_bytes())
            .map_err(|_| invalid("Journalfeld ist nicht versiegelt"))?;
        let plain = decrypt_bytes(&self.cryptor, &bytes, MAX_FIELD)?;
        std::str::from_utf8(&plain)
            .map(str::to_owned)
            .map_err(|_| invalid("Versiegeltes Journalfeld ist kein Text"))
    }
}

impl PathSealer for JournalPathSealer {
    fn seal(&self, path: &str) -> io::Result<String> {
        self.seal_text(path)
    }

    fn unseal(&self, sealed: &str) -> io::Result<String> {
        self.unseal_text(sealed)
    }

    fn seal_detail(&self, text: &str) -> io::Result<String> {
        self.seal_text(bounded(text))
    }

    fn unseal_detail(&self, sealed: &str) -> io::Result<String> {
        self.unseal_text(sealed)
    }
}

/// The spool and path protection of a vault drive (contract V1).
pub(crate) fn sealed_mount_storage(key: &SpoolKey) -> io::Result<MountStorage> {
    Ok(MountStorage {
        spool: Arc::new(SealedSpoolStorage::new(key)?),
        paths: Arc::new(JournalPathSealer::new(key)?),
    })
}

/// `text` cut to `MAX_FIELD` bytes at a character boundary.
fn bounded(text: &str) -> &str {
    if text.len() <= MAX_FIELD {
        return text;
    }
    let mut end = MAX_FIELD;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
