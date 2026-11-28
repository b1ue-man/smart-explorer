//! The sealed files of the replace journal: every file (a run's record, a
//! file's record, the original of a replaced file) is encrypted with
//! ChaCha20-Poly1305 under a key derived (HKDF-SHA256) from the journal key
//! and a random salt of its own, in chunks of 1 MiB. The associated data of
//! each chunk binds the format, the salt, the file's label (run id, entry
//! number, kind), the chunk number and whether it is the last, so no file can
//! stand in for another, be cut short or be extended.
//!
//! Layout: magic (8 bytes), salt (16), then per chunk a flag byte (1 = last),
//! the sealed length (u32, little endian) and the sealed bytes (text + tag).
use std::io::{self, Read, Write};
use std::path::Path;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::hkdf::{Salt, HKDF_SHA256};
use ring::rand::{SecureRandom, SystemRandom};

const MAGIC: &[u8; 8] = b"SERPJ01\0";
const SALT_BYTES: usize = 16;
const CHUNK_BYTES: usize = 1024 * 1024;
const TAG_BYTES: usize = 16;
const KEY_INFO: &[u8] = b"smart-explorer/replace-journal/v1";

/// The journal key: 32 bytes, only in memory (and in the secret store).
#[derive(Clone)]
pub(super) struct JournalKey {
    bytes: [u8; 32],
}

impl std::fmt::Debug for JournalKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JournalKey([REDACTED])")
    }
}

impl Drop for JournalKey {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

impl JournalKey {
    pub(super) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// A new random key.
    pub(super) fn generate() -> io::Result<Self> {
        let mut bytes = [0u8; 32];
        random(&mut bytes)?;
        Ok(Self { bytes })
    }

    pub(super) fn to_hex(&self) -> String {
        self.bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(super) fn from_hex(text: &str) -> io::Result<Self> {
        let text = text.trim();
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "ungültiger Journalschlüssel");
        if text.len() != 64 || !text.is_ascii() {
            return Err(invalid());
        }
        let mut bytes = [0u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte =
                u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).map_err(|_| invalid())?;
        }
        Ok(Self { bytes })
    }

    fn derive(&self, salt: &[u8]) -> io::Result<LessSafeKey> {
        let refused = || io::Error::other("der Journalschlüssel wurde abgelehnt");
        let prk = Salt::new(HKDF_SHA256, salt).extract(&self.bytes);
        let okm = prk
            .expand(&[KEY_INFO], &CHACHA20_POLY1305)
            .map_err(|_| refused())?;
        Ok(LessSafeKey::new(UnboundKey::from(okm)))
    }
}

fn random(bytes: &mut [u8]) -> io::Result<()> {
    SystemRandom::new()
        .fill(bytes)
        .map_err(|_| io::Error::other("keine Zufallszahlen für das Ersetzungsjournal"))
}

fn nonce(index: u64) -> Nonce {
    let mut bytes = [0u8; NONCE_LEN];
    bytes[..8].copy_from_slice(&index.to_le_bytes());
    Nonce::assume_unique_for_key(bytes)
}

fn aad(salt: &[u8], label: &str, index: u64, last: bool) -> Vec<u8> {
    let mut aad = Vec::with_capacity(MAGIC.len() + salt.len() + label.len() + 16);
    aad.extend_from_slice(MAGIC);
    aad.extend_from_slice(salt);
    aad.extend_from_slice(&(label.len() as u64).to_le_bytes());
    aad.extend_from_slice(label.as_bytes());
    aad.extend_from_slice(&index.to_le_bytes());
    aad.push(u8::from(last));
    aad
}

/// Writes `plain` sealed under `key` and `label` into `out`.
pub(super) fn seal_into(
    key: &JournalKey,
    label: &str,
    plain: &[u8],
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut salt = [0u8; SALT_BYTES];
    random(&mut salt)?;
    let sealing = key.derive(&salt)?;
    out.write_all(MAGIC)?;
    out.write_all(&salt)?;
    let count = plain.len().div_ceil(CHUNK_BYTES).max(1);
    let mut buffer = Vec::with_capacity(CHUNK_BYTES.min(plain.len()) + TAG_BYTES);
    for index in 0..count {
        let start = index * CHUNK_BYTES;
        let chunk = &plain[start.min(plain.len())..(start + CHUNK_BYTES).min(plain.len())];
        let last = index + 1 == count;
        buffer.clear();
        buffer.extend_from_slice(chunk);
        sealing
            .seal_in_place_append_tag(
                nonce(index as u64),
                Aad::from(aad(&salt, label, index as u64, last)),
                &mut buffer,
            )
            .map_err(|_| io::Error::other("Verschlüsseln fehlgeschlagen"))?;
        out.write_all(&[u8::from(last)])?;
        out.write_all(&(buffer.len() as u32).to_le_bytes())?;
        out.write_all(&buffer)?;
    }
    Ok(())
}

/// `plain` sealed as one byte vector.
pub(super) fn seal_bytes(key: &JournalKey, label: &str, plain: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(plain.len() + 64);
    seal_into(key, label, plain, &mut out)?;
    Ok(out)
}

/// Opens what `seal_into` wrote; at most `limit` plain bytes.
pub(super) fn open_from(
    key: &JournalKey,
    label: &str,
    input: &mut dyn Read,
    limit: u64,
) -> io::Result<Vec<u8>> {
    let broken = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut head = [0u8; 8 + SALT_BYTES];
    input
        .read_exact(&mut head)
        .map_err(|_| broken("Journaldatei zu kurz"))?;
    if &head[..8] != MAGIC {
        return Err(broken("keine Journaldatei"));
    }
    let salt = &head[8..];
    let opening = key.derive(salt)?;
    let mut plain = Vec::new();
    let mut index = 0u64;
    loop {
        let mut frame = [0u8; 5];
        input
            .read_exact(&mut frame)
            .map_err(|_| broken("Journaldatei abgeschnitten"))?;
        let last = match frame[0] {
            0 => false,
            1 => true,
            _ => return Err(broken("Journaldatei beschädigt")),
        };
        let length = u32::from_le_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        if !(TAG_BYTES..=CHUNK_BYTES + TAG_BYTES).contains(&length) {
            return Err(broken("Journaldatei beschädigt"));
        }
        if plain.len() as u64 + (length - TAG_BYTES) as u64 > limit {
            return Err(broken("Journaldatei größer als erwartet"));
        }
        let mut buffer = vec![0u8; length];
        input
            .read_exact(&mut buffer)
            .map_err(|_| broken("Journaldatei abgeschnitten"))?;
        let opened = opening
            .open_in_place(
                nonce(index),
                Aad::from(aad(salt, label, index, last)),
                &mut buffer,
            )
            .map_err(|_| {
                broken("Journaldatei lässt sich nicht entschlüsseln (Schlüssel oder Inhalt passt nicht)")
            })?;
        plain.extend_from_slice(opened);
        index += 1;
        if last {
            break;
        }
    }
    let mut rest = [0u8; 1];
    if input.read(&mut rest)? != 0 {
        return Err(broken("Journaldatei hat Daten nach dem Ende"));
    }
    Ok(plain)
}

/// Seals `plain` into the new private file `path` (it must not exist).
pub(super) fn seal_new_file(
    key: &JournalKey,
    label: &str,
    plain: &[u8],
    path: &Path,
) -> io::Result<()> {
    let file = crate::support_dirs::create_private_file(path)?;
    let mut writer = io::BufWriter::new(file);
    let written = seal_into(key, label, plain, &mut writer)
        .and_then(|()| writer.flush())
        .and_then(|()| writer.get_ref().sync_all());
    if written.is_err() {
        drop(writer);
        let _ = std::fs::remove_file(path);
    }
    written
}

/// Replaces the private file `path` atomically with `plain` sealed.
pub(super) fn seal_replace_file(
    key: &JournalKey,
    label: &str,
    plain: &[u8],
    path: &Path,
) -> io::Result<()> {
    let sealed = seal_bytes(key, label, plain)?;
    crate::support_dirs::write_private_atomic(path, &sealed)
}

/// Opens the sealed private file `path`.
pub(super) fn open_file(
    key: &JournalKey,
    label: &str,
    path: &Path,
    limit: u64,
) -> io::Result<Vec<u8>> {
    let file = crate::support_dirs::open_private_file(path)?;
    open_from(key, label, &mut io::BufReader::new(file), limit)
}
