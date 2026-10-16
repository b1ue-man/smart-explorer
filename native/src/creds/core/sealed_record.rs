//! On-disk records of the file credential store (Linux and Android).
//!
//! Format 1 (`SESEC01`) kept the secret in plaintext behind a SHA-256
//! checksum. Format 2 (`SESEC02`) seals it with ChaCha20-Poly1305 under the
//! store's 32-byte data key; magic, version, account digest and key id are the
//! associated data, so a record can neither be moved to another account nor
//! opened with another key. Format 1 is only read, to port existing stores.
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(crate) const ACCOUNT_DIGEST_BYTES: usize = 32;
pub(crate) const MAX_SECRET_BYTES: usize = 64 * 1024;
pub(crate) const KEY_BYTES: usize = 32;
pub(crate) const KEY_ID_BYTES: usize = 16;

const LEGACY_MAGIC: &[u8; 8] = b"SESEC01\0";
const LEGACY_VERSION: u8 = 1;
const LEGACY_CHECKSUM_BYTES: usize = 32;
const LEGACY_HEADER_BYTES: usize = LEGACY_MAGIC.len() + 1 + ACCOUNT_DIGEST_BYTES + 4;

const SEALED_MAGIC: &[u8; 8] = b"SESEC02\0";
const SEALED_VERSION: u8 = 2;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
/// Magic, version, account digest and key id: the associated data.
const SEALED_AAD_BYTES: usize = SEALED_MAGIC.len() + 1 + ACCOUNT_DIGEST_BYTES + KEY_ID_BYTES;
const SEALED_HEADER_BYTES: usize = SEALED_AAD_BYTES + NONCE_BYTES + 4;

/// Largest record of either format.
pub(crate) const MAX_RECORD_BYTES: usize = SEALED_HEADER_BYTES + MAX_SECRET_BYTES + TAG_BYTES;

const KEY_ID_DOMAIN: &[u8] = b"smart-explorer/secrets/key-id/v2\0";
const WRAP_AAD: &[u8] = b"smart-explorer/secrets/data-key-wrap/v2";
/// Wrapped data key: nonce, sealed key, tag.
pub(crate) const WRAPPED_KEY_BYTES: usize = NONCE_BYTES + KEY_BYTES + TAG_BYTES;

/// Name of the descriptor file inside the store directory.
pub(crate) const DESCRIPTOR_NAME: &str = "keyring-v2";
pub(crate) const DESCRIPTOR_FORMAT: u8 = 2;
pub(crate) const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;

/// The store's data key and its public id (first 16 bytes of a domain
/// separated SHA-256), which records and the descriptor name.
#[derive(Clone)]
pub(crate) struct DataKey {
    id: [u8; KEY_ID_BYTES],
    key: Zeroizing<[u8; KEY_BYTES]>,
}

impl std::fmt::Debug for DataKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DataKey")
            .field("id", &hex(&self.id))
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl DataKey {
    pub(crate) fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        let key = Zeroizing::new(bytes);
        let mut hasher = Sha256::new();
        hasher.update(KEY_ID_DOMAIN);
        hasher.update(&key[..]);
        let digest = hasher.finalize();
        let mut id = [0u8; KEY_ID_BYTES];
        id.copy_from_slice(&digest[..KEY_ID_BYTES]);
        Self { id, key }
    }

    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self, String> {
        let bytes: [u8; KEY_BYTES] = bytes
            .try_into()
            .map_err(|_| "credential store: data key must be 32 bytes".to_string())?;
        Ok(Self::from_bytes(bytes))
    }

    pub(crate) fn generate() -> Result<Self, String> {
        let mut bytes = Zeroizing::new([0u8; KEY_BYTES]);
        getrandom::getrandom(&mut bytes[..])
            .map_err(|error| format!("credential store: generate data key: {error}"))?;
        Ok(Self::from_bytes(*bytes))
    }

    pub(crate) fn id(&self) -> &[u8; KEY_ID_BYTES] {
        &self.id
    }

    pub(crate) fn id_hex(&self) -> String {
        hex(&self.id)
    }

    pub(crate) fn expose(&self) -> &[u8; KEY_BYTES] {
        &self.key
    }
}

/// What a record file holds, decided from its first bytes only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecordKind {
    /// Format 1: plaintext, still to be ported.
    Legacy,
    /// Format 2: sealed under the data key with this id.
    Sealed([u8; KEY_ID_BYTES]),
}

pub(crate) fn account_digest(account: &str) -> [u8; ACCOUNT_DIGEST_BYTES] {
    let mut hasher = Sha256::new();
    hasher.update(b"smart_explorer\0");
    hasher.update(account.as_bytes());
    hasher.finalize().into()
}

pub(crate) fn record_name_for_digest(digest: &[u8; ACCOUNT_DIGEST_BYTES]) -> String {
    format!("{}.secret", hex(digest))
}

/// The account digest a record file name stands for (`<64 hex>.secret`).
pub(crate) fn digest_from_record_name(name: &str) -> Option<[u8; ACCOUNT_DIGEST_BYTES]> {
    let stem = name.strip_suffix(".secret")?;
    if stem.len() != ACCOUNT_DIGEST_BYTES * 2 {
        return None;
    }
    let mut digest = [0u8; ACCOUNT_DIGEST_BYTES];
    for (index, slot) in digest.iter_mut().enumerate() {
        *slot = u8::from_str_radix(stem.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    (hex(&digest) == stem).then_some(digest)
}

pub(crate) fn record_kind(record: &[u8]) -> Result<RecordKind, String> {
    if record.len() > MAX_RECORD_BYTES {
        return Err(format!(
            "Linux credential store: record exceeds {MAX_RECORD_BYTES} bytes"
        ));
    }
    if record.len() < LEGACY_MAGIC.len() + 1 {
        return Err("Linux credential store: truncated credential record".into());
    }
    let magic = &record[..LEGACY_MAGIC.len()];
    let version = record[LEGACY_MAGIC.len()];
    if magic == LEGACY_MAGIC {
        return if version == LEGACY_VERSION {
            Ok(RecordKind::Legacy)
        } else {
            Err("Linux credential store: unsupported credential record version".into())
        };
    }
    if magic != SEALED_MAGIC {
        return Err("Linux credential store: invalid credential record magic".into());
    }
    if version != SEALED_VERSION {
        return Err("Linux credential store: unsupported credential record version".into());
    }
    if record.len() < SEALED_HEADER_BYTES + TAG_BYTES {
        return Err("Linux credential store: truncated credential record".into());
    }
    let start = SEALED_MAGIC.len() + 1 + ACCOUNT_DIGEST_BYTES;
    let mut key_id = [0u8; KEY_ID_BYTES];
    key_id.copy_from_slice(&record[start..start + KEY_ID_BYTES]);
    Ok(RecordKind::Sealed(key_id))
}

/// Seals `secret` for the account with `digest` under `key`.
pub(crate) fn seal(
    digest: &[u8; ACCOUNT_DIGEST_BYTES],
    secret: &[u8],
    key: &DataKey,
) -> Result<Vec<u8>, String> {
    if secret.len() > MAX_SECRET_BYTES {
        return Err(format!(
            "Linux credential store: secret exceeds {MAX_SECRET_BYTES} bytes"
        ));
    }
    let mut nonce = [0u8; NONCE_BYTES];
    getrandom::getrandom(&mut nonce)
        .map_err(|error| format!("Linux credential store: generate record nonce: {error}"))?;
    let mut record = Vec::with_capacity(SEALED_HEADER_BYTES + secret.len() + TAG_BYTES);
    record.extend_from_slice(SEALED_MAGIC);
    record.push(SEALED_VERSION);
    record.extend_from_slice(digest);
    record.extend_from_slice(key.id());
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key.expose()[..]));
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret,
                aad: &record[..SEALED_AAD_BYTES],
            },
        )
        .map_err(|_| "Linux credential store: seal credential record".to_string())?;
    record.extend_from_slice(&nonce);
    record.extend_from_slice(&(sealed.len() as u32).to_be_bytes());
    record.extend_from_slice(&sealed);
    Ok(record)
}

/// Opens a format 2 record of the account with `digest`.
pub(crate) fn open_sealed(
    digest: &[u8; ACCOUNT_DIGEST_BYTES],
    record: &[u8],
    key: &DataKey,
) -> Result<Zeroizing<Vec<u8>>, String> {
    match record_kind(record)? {
        RecordKind::Sealed(id) if id == *key.id() => {}
        RecordKind::Sealed(_) => {
            return Err(
                "Linux credential store: credential record was sealed with another data key".into(),
            )
        }
        RecordKind::Legacy => {
            return Err("Linux credential store: expected a sealed credential record".into())
        }
    }
    let digest_start = SEALED_MAGIC.len() + 1;
    if record[digest_start..digest_start + ACCOUNT_DIGEST_BYTES] != digest[..] {
        return Err("Linux credential store: credential record belongs to another account".into());
    }
    let nonce = &record[SEALED_AAD_BYTES..SEALED_AAD_BYTES + NONCE_BYTES];
    let length_start = SEALED_AAD_BYTES + NONCE_BYTES;
    let length = u32::from_be_bytes(
        record[length_start..length_start + 4]
            .try_into()
            .map_err(|_| "Linux credential store: invalid secret length".to_string())?,
    ) as usize;
    if length > MAX_SECRET_BYTES + TAG_BYTES {
        return Err(format!(
            "Linux credential store: secret exceeds {MAX_SECRET_BYTES} bytes"
        ));
    }
    if length < TAG_BYTES || record.len() != SEALED_HEADER_BYTES + length {
        return Err("Linux credential store: credential record length mismatch".into());
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key.expose()[..]));
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &record[SEALED_HEADER_BYTES..],
                aad: &record[..SEALED_AAD_BYTES],
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| "Linux credential store: credential record failed authentication".into())
}

/// Reads a format 1 record of the account with `digest`.
pub(crate) fn open_legacy(
    digest: &[u8; ACCOUNT_DIGEST_BYTES],
    record: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    if record_kind(record)? != RecordKind::Legacy {
        return Err("Linux credential store: expected a legacy credential record".into());
    }
    if record.len() < LEGACY_HEADER_BYTES + LEGACY_CHECKSUM_BYTES {
        return Err("Linux credential store: truncated credential record".into());
    }
    let digest_start = LEGACY_MAGIC.len() + 1;
    let digest_end = digest_start + ACCOUNT_DIGEST_BYTES;
    if record[digest_start..digest_end] != digest[..] {
        return Err("Linux credential store: credential record belongs to another account".into());
    }
    let length = u32::from_be_bytes(
        record[digest_end..digest_end + 4]
            .try_into()
            .map_err(|_| "Linux credential store: invalid secret length".to_string())?,
    ) as usize;
    if length > MAX_SECRET_BYTES {
        return Err(format!(
            "Linux credential store: secret exceeds {MAX_SECRET_BYTES} bytes"
        ));
    }
    let expected = LEGACY_HEADER_BYTES + length + LEGACY_CHECKSUM_BYTES;
    if record.len() != expected {
        return Err("Linux credential store: credential record length mismatch".into());
    }
    let checksum_start = expected - LEGACY_CHECKSUM_BYTES;
    let checksum = Sha256::digest(&record[..checksum_start]);
    if record[checksum_start..] != checksum[..] {
        return Err("Linux credential store: credential record checksum mismatch".into());
    }
    Ok(Zeroizing::new(
        record[LEGACY_HEADER_BYTES..checksum_start].to_vec(),
    ))
}

/// A format 1 record as earlier versions wrote it (tests and porting checks).
#[cfg(test)]
pub(crate) fn encode_legacy(account: &str, secret: &str) -> Vec<u8> {
    let mut record = Vec::with_capacity(LEGACY_HEADER_BYTES + secret.len() + 32);
    record.extend_from_slice(LEGACY_MAGIC);
    record.push(LEGACY_VERSION);
    record.extend_from_slice(&account_digest(account));
    record.extend_from_slice(&(secret.len() as u32).to_be_bytes());
    record.extend_from_slice(secret.as_bytes());
    let checksum = Sha256::digest(&record);
    record.extend_from_slice(&checksum);
    record
}

/// Seals the data key under a key-encryption key (the `app-key-file` protector).
pub(crate) fn wrap_key(kek: &[u8; KEY_BYTES], key: &DataKey) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; NONCE_BYTES];
    getrandom::getrandom(&mut nonce)
        .map_err(|error| format!("credential store: generate wrap nonce: {error}"))?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&kek[..]));
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &key.expose()[..],
                aad: WRAP_AAD,
            },
        )
        .map_err(|_| "credential store: wrap data key".to_string())?;
    let mut wrapped = Vec::with_capacity(WRAPPED_KEY_BYTES);
    wrapped.extend_from_slice(&nonce);
    wrapped.extend_from_slice(&sealed);
    Ok(wrapped)
}

pub(crate) fn unwrap_key(kek: &[u8; KEY_BYTES], wrapped: &[u8]) -> Result<DataKey, String> {
    if wrapped.len() != WRAPPED_KEY_BYTES {
        return Err("credential store: wrapped data key has the wrong length".into());
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&kek[..]));
    let opened = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&wrapped[..NONCE_BYTES]),
                Payload {
                    msg: &wrapped[NONCE_BYTES..],
                    aad: WRAP_AAD,
                },
            )
            .map_err(|_| {
                "credential store: data key does not open with this key file".to_string()
            })?,
    );
    DataKey::from_slice(&opened)
}

/// `secrets-v1/keyring-v2`: which protector holds the data key and its id.
/// `wrapped` is protector-specific (empty for the Android Keystore, whose
/// app-side wrap lives outside the store).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct KeyringDescriptor {
    pub(crate) format: u8,
    pub(crate) protector: String,
    pub(crate) key_id: String,
    #[serde(default)]
    pub(crate) wrapped: String,
}

impl KeyringDescriptor {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err("credential store: key descriptor exceeds 64 KiB".into());
        }
        let descriptor: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("credential store: key descriptor: {error}"))?;
        if descriptor.format != DESCRIPTOR_FORMAT
            || descriptor.protector.is_empty()
            || descriptor.key_id.len() != KEY_ID_BYTES * 2
        {
            return Err("credential store: unsupported key descriptor".into());
        }
        Ok(descriptor)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self)
            .map_err(|error| format!("credential store: key descriptor: {error}"))
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(crate) fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
#[path = "sealed_record_tests.rs"]
mod tests;
