//! Spool keys of vault drives (C3.6, contract V1, S2-V2, VLT-M12).
//!
//! Every vault drive has its own random 32-byte spool key. The worker wraps
//! it with the purpose key `spool-wrap` of the vault (AES-KW, RFC 3394) and
//! keeps the 40 wrapped bytes as `<spool-root>/<mount-id>/spool-key.wrapped`;
//! after a crash it unwraps the same key again once the vault is unlocked.
//! The masterkey never changes on a password change, so neither does the
//! spool key. The drive host receives only the spool key (`Vault::SpoolKey`),
//! never the masterkey.
//!
//! The spool key encrypts nothing itself. HKDF-SHA256 derives two keys of
//! 64 bytes (encryption half ‖ MAC half of the vault content format) from
//! it: the content key (`se-mount-spool-content-v1`) of the sealed spool
//! files and the text key (`se-mount-journal-v1`) of the journal fields.
//! Both seal in the content format `SIV_GCM` (header plus AES-256-GCM
//! chunks); the MAC half is derived for the shape of the key only.
use std::fmt;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use aes_kw::{KeyInit, KwAes256};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::cryptomator::{CipherCombo, ContentCryptor, Masterkey, MasterkeyCell};

/// The wrapped spool key beside the journal of a drive.
pub(crate) const SPOOL_KEY_FILE: &str = "spool-key.wrapped";
/// Length of a spool key.
pub(crate) const SPOOL_KEY_LEN: usize = 32;
/// Length of an AES-KW-wrapped spool key.
pub(crate) const WRAPPED_LEN: usize = SPOOL_KEY_LEN + 8;
/// HKDF `info` of the content key of sealed spool files.
pub(crate) const CONTENT_INFO: &str = "se-mount-spool-content-v1";
/// HKDF `info` of the text key of sealed journal fields.
pub(crate) const TEXT_INFO: &str = "se-mount-journal-v1";
/// Length of a derived key: encryption half ‖ MAC half.
const DERIVED_LEN: usize = 64;

/// One worker writes key files at a time, so two hosts of one drive never
/// get two different keys.
static CREATE: Mutex<()> = Mutex::new(());

/// The spool key of one vault drive; wiped when dropped, never printed.
pub(crate) struct SpoolKey(Zeroizing<[u8; SPOOL_KEY_LEN]>);

impl SpoolKey {
    /// A new random key.
    pub(crate) fn generate() -> io::Result<Self> {
        let mut key = Zeroizing::new([0u8; SPOOL_KEY_LEN]);
        getrandom::getrandom(&mut key[..])
            .map_err(|error| io::Error::other(format!("Zufallszahlen: {error}")))?;
        Ok(Self(key))
    }

    /// The key from its 32 bytes (the reply of `Vault::SpoolKey`).
    pub(crate) fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let bytes: &[u8; SPOOL_KEY_LEN] = bytes
            .try_into()
            .map_err(|_| invalid("Spool-Schlüssel hat nicht 32 Byte"))?;
        Ok(Self(Zeroizing::new(*bytes)))
    }

    /// The raw key, for the one IPC reply that hands it to the drive host.
    pub(crate) fn expose(&self) -> &[u8; SPOOL_KEY_LEN] {
        &self.0
    }

    /// HKDF-SHA256 without salt over the spool key, `info` of the purpose.
    pub(crate) fn derive(&self, info: &str) -> io::Result<Zeroizing<[u8; DERIVED_LEN]>> {
        let mut out = Zeroizing::new([0u8; DERIVED_LEN]);
        Hkdf::<Sha256>::new(None, &self.0[..])
            .expand(info.as_bytes(), &mut out[..])
            .map_err(|_| io::Error::other("HKDF des Spool-Schlüssels fehlgeschlagen"))?;
        Ok(out)
    }

    /// The content format under the content key (`SealedSpoolFile`).
    pub(crate) fn content_cryptor(&self) -> io::Result<ContentCryptor> {
        self.cryptor(CONTENT_INFO)
    }

    /// The content format under the text key (`JournalPathSealer`).
    pub(crate) fn text_cryptor(&self) -> io::Result<ContentCryptor> {
        self.cryptor(TEXT_INFO)
    }

    fn cryptor(&self, info: &str) -> io::Result<ContentCryptor> {
        let raw = self.derive(info)?;
        let key = Masterkey::from_raw(&raw);
        Ok(ContentCryptor::new(
            MasterkeyCell::new(key),
            CipherCombo::SivGcm,
        ))
    }
}

impl fmt::Debug for SpoolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SpoolKey(..)")
    }
}

/// AES-KW of `key` with the wrap key of the vault.
pub(crate) fn wrap(wrap_key: &[u8; 32], key: &SpoolKey) -> io::Result<[u8; WRAPPED_LEN]> {
    let wrapper = KwAes256::new_from_slice(wrap_key)
        .map_err(|_| io::Error::other("AES-KW-Schlüssel ungültig"))?;
    let mut wrapped = [0u8; WRAPPED_LEN];
    wrapper
        .wrap_key(key.expose(), &mut wrapped)
        .map_err(|_| io::Error::other("Spool-Schlüssel wickeln fehlgeschlagen"))?;
    Ok(wrapped)
}

/// The key inside `wrapped`; another vault (or changed bytes) yields no key.
pub(crate) fn unwrap(wrap_key: &[u8; 32], wrapped: &[u8; WRAPPED_LEN]) -> io::Result<SpoolKey> {
    let wrapper = KwAes256::new_from_slice(wrap_key)
        .map_err(|_| io::Error::other("AES-KW-Schlüssel ungültig"))?;
    let mut key = Zeroizing::new([0u8; SPOOL_KEY_LEN]);
    wrapper
        .unwrap_key(wrapped, &mut key[..])
        .map_err(|_| invalid("Der Spool-Schlüssel des Laufwerks passt nicht zu diesem Tresor"))?;
    Ok(SpoolKey(key))
}

/// The spool key of the drive whose cache folder is `mount_dir`: the stored
/// one unwrapped, else a new one, wrapped and stored durably before it is
/// returned. A stored key that does not unwrap is an error and stays: sealed
/// changes of the drive may depend on it.
pub(crate) fn load_or_create(mount_dir: &Path, wrap_key: &[u8; 32]) -> io::Result<SpoolKey> {
    let _guard = CREATE.lock().unwrap_or_else(PoisonError::into_inner);
    let path = mount_dir.join(SPOOL_KEY_FILE);
    if let Some(wrapped) = read_wrapped(&path)? {
        return unwrap(wrap_key, &wrapped);
    }
    let key = SpoolKey::generate()?;
    let wrapped = wrap(wrap_key, &key)?;
    let mut file = crate::support_dirs::create_private_file(&path)?;
    let written = file.write_all(&wrapped).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) =
        written.and_then(|()| crate::creds::private_storage::sync_directory(mount_dir))
    {
        // A key that is not durable is never handed out.
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(key)
}

/// The stored wrapped key; `None` when the drive has none yet.
fn read_wrapped(path: &Path) -> io::Result<Option<[u8; WRAPPED_LEN]>> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(metadata) if !metadata.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Spool-Schlüsseldatei ist keine einfache Datei",
            ))
        }
        Ok(_) => {}
    }
    let file = crate::support_dirs::open_private_file(path)?;
    if file.metadata()?.len() != WRAPPED_LEN as u64 {
        return Err(invalid("Spool-Schlüsseldatei hat nicht 40 Byte"));
    }
    let mut wrapped = [0u8; WRAPPED_LEN];
    file.take(WRAPPED_LEN as u64).read_exact(&mut wrapped)?;
    Ok(Some(wrapped))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
