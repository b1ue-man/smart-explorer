//! The header in front of every encrypted file: nonce ‖ encrypted payload ‖
//! tag, where the payload is 8 reserved bytes (`-1` big-endian when written,
//! ignored when read, as CryptoLib does) ‖ the file's 32-byte content key.
//! `SIV_GCM`: AES-256-GCM with the encryption masterkey, 68 bytes.
//! `SIV_CTRMAC`: AES-256-CTR with the encryption masterkey, then
//! HMAC-SHA256(MAC masterkey, nonce ‖ ciphertext), 88 bytes.
use std::fmt;

use zeroize::Zeroizing;

use super::cipher_combo::{CipherCombo, HEADER_PAYLOAD};
use super::content_chunks::{ctr_apply, gcm_open, gcm_seal, hmac_sha256, hmac_sha256_matches};
use super::keys::{fill_random, Masterkey, SUBKEY_LEN};
use super::FormatError;

/// The reserved payload field as CryptoLib writes it (`-1L`).
const RESERVED: [u8; 8] = [0xFF; 8];

/// The per-file nonce and content key of one encrypted file.
pub(crate) struct FileHeader {
    combo: CipherCombo,
    nonce: Vec<u8>,
    content_key: Zeroizing<[u8; SUBKEY_LEN]>,
}

impl FileHeader {
    /// A header for a new file: random nonce and random content key.
    pub(crate) fn generate(combo: CipherCombo) -> Result<Self, FormatError> {
        let mut nonce = vec![0u8; combo.nonce_len()];
        fill_random(&mut nonce)?;
        let mut content_key = Zeroizing::new([0u8; SUBKEY_LEN]);
        fill_random(&mut content_key[..])?;
        Ok(Self {
            combo,
            nonce,
            content_key,
        })
    }

    /// A header from known parts (decryption result, CryptoLib vectors).
    pub(crate) fn from_parts(
        combo: CipherCombo,
        nonce: &[u8],
        content_key: Zeroizing<[u8; SUBKEY_LEN]>,
    ) -> Result<Self, FormatError> {
        if nonce.len() != combo.nonce_len() {
            return Err(FormatError::InvalidHeader);
        }
        Ok(Self {
            combo,
            nonce: nonce.to_vec(),
            content_key,
        })
    }

    pub(crate) fn combo(&self) -> CipherCombo {
        self.combo
    }

    pub(crate) fn nonce(&self) -> &[u8] {
        &self.nonce
    }

    pub(crate) fn content_key(&self) -> &[u8; SUBKEY_LEN] {
        &self.content_key
    }
}

impl fmt::Debug for FileHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileHeader")
            .field("combo", &self.combo)
            .finish_non_exhaustive()
    }
}

/// The header bytes of `header`, encrypted with the masterkey.
pub(crate) fn encrypt_header(key: &Masterkey, header: &FileHeader) -> Result<Vec<u8>, FormatError> {
    let combo = header.combo;
    let mut payload = Zeroizing::new([0u8; HEADER_PAYLOAD]);
    payload[..RESERVED.len()].copy_from_slice(&RESERVED);
    payload[RESERVED.len()..].copy_from_slice(&header.content_key[..]);
    let mut out = Vec::with_capacity(combo.header_size());
    out.extend_from_slice(&header.nonce);
    match combo {
        CipherCombo::SivGcm => {
            let tag = gcm_seal(key.enc_key(), &header.nonce, &[], &mut payload[..])?;
            out.extend_from_slice(&payload[..]);
            out.extend_from_slice(&tag);
        }
        CipherCombo::SivCtrMac => {
            ctr_apply(key.enc_key(), &header.nonce, &mut payload[..])?;
            let mac = hmac_sha256(key.mac_key(), &[header.nonce.as_slice(), &payload[..]])?;
            out.extend_from_slice(&payload[..]);
            out.extend_from_slice(&mac);
        }
    }
    Ok(out)
}

/// Authenticates and decrypts a header of `combo`; `bytes` must be exactly
/// the header (68 or 88 bytes).
pub(crate) fn decrypt_header(
    key: &Masterkey,
    combo: CipherCombo,
    bytes: &[u8],
) -> Result<FileHeader, FormatError> {
    if bytes.len() != combo.header_size() {
        return Err(FormatError::InvalidHeader);
    }
    let nonce = &bytes[..combo.nonce_len()];
    let payload_end = combo.nonce_len() + HEADER_PAYLOAD;
    let tag = &bytes[payload_end..];
    let mut payload = Zeroizing::new([0u8; HEADER_PAYLOAD]);
    payload.copy_from_slice(&bytes[combo.nonce_len()..payload_end]);
    match combo {
        CipherCombo::SivGcm => {
            gcm_open(
                key.enc_key(),
                nonce,
                &[],
                &mut payload[..],
                tag,
                "Dateikopf",
            )?;
        }
        CipherCombo::SivCtrMac => {
            if !hmac_sha256_matches(key.mac_key(), &[nonce, &payload[..]], tag)? {
                return Err(FormatError::Authentication("Dateikopf"));
            }
            ctr_apply(key.enc_key(), nonce, &mut payload[..])?;
        }
    }
    let mut content_key = Zeroizing::new([0u8; SUBKEY_LEN]);
    content_key.copy_from_slice(&payload[RESERVED.len()..]);
    FileHeader::from_parts(combo, nonce, content_key)
}
