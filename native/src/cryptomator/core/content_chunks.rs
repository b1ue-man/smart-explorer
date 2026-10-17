//! One chunk of file content and the primitives both cipher combinations
//! share: AES-256-GCM (`SIV_GCM`) and AES-256-CTR with HMAC-SHA256
//! (`SIV_CTRMAC`). Every tag or MAC check runs in constant time and comes
//! before any plaintext is released; a failed check returns an error and
//! wipes the working buffer.
use hmac_013::{Hmac, KeyInit, Mac};
use zeroize::{Zeroize, Zeroizing};

use super::cipher_combo::{CipherCombo, CHUNK_PAYLOAD};
use super::content_header::FileHeader;
use super::keys::{fill_random, Masterkey, SUBKEY_LEN};
use super::FormatError;

type HmacSha256 = Hmac<sha2_011::Sha256>;

const GCM_NONCE: usize = 12;
const CTR_IV: usize = 16;

/// AES-256-GCM encryption of `data` in place; returns the 16-byte tag.
pub(super) fn gcm_seal(
    key: &[u8; SUBKEY_LEN],
    nonce: &[u8],
    aad: &[u8],
    data: &mut [u8],
) -> Result<[u8; 16], FormatError> {
    use aes_gcm_011::AeadInOut;
    let nonce: [u8; GCM_NONCE] = nonce
        .try_into()
        .map_err(|_| FormatError::Crypto("AES-GCM"))?;
    let cipher =
        aes_gcm_011::Aes256Gcm::new_from_slice(key).map_err(|_| FormatError::Crypto("AES-GCM"))?;
    let tag = cipher
        .encrypt_inout_detached(&nonce.into(), aad, data.into())
        .map_err(|_| FormatError::Crypto("AES-GCM"))?;
    let mut out = [0u8; 16];
    out.copy_from_slice(&tag);
    Ok(out)
}

/// AES-256-GCM decryption of `data` in place after the tag verified; on a
/// failed check `data` is wiped and nothing is decrypted.
pub(super) fn gcm_open(
    key: &[u8; SUBKEY_LEN],
    nonce: &[u8],
    aad: &[u8],
    data: &mut [u8],
    tag: &[u8],
    what: &'static str,
) -> Result<(), FormatError> {
    use aes_gcm_011::AeadInOut;
    let nonce: [u8; GCM_NONCE] = nonce
        .try_into()
        .map_err(|_| FormatError::Authentication(what))?;
    let tag = <aes_gcm_011::Tag>::try_from(tag).map_err(|_| FormatError::Authentication(what))?;
    let cipher =
        aes_gcm_011::Aes256Gcm::new_from_slice(key).map_err(|_| FormatError::Crypto("AES-GCM"))?;
    let opened = cipher.decrypt_inout_detached(&nonce.into(), aad, (&mut *data).into(), &tag);
    if opened.is_err() {
        data.zeroize();
        return Err(FormatError::Authentication(what));
    }
    Ok(())
}

/// AES-256-CTR with a 128-bit big-endian counter starting at `iv` (Java
/// `AES/CTR/NoPadding`), applied in place.
pub(super) fn ctr_apply(
    key: &[u8; SUBKEY_LEN],
    iv: &[u8],
    data: &mut [u8],
) -> Result<(), FormatError> {
    use ctr_010::cipher::{KeyIvInit, StreamCipher};
    if iv.len() != CTR_IV {
        return Err(FormatError::Crypto("AES-CTR"));
    }
    let mut cipher = ctr_010::Ctr128BE::<aes_09::Aes256>::new_from_slices(key, iv)
        .map_err(|_| FormatError::Crypto("AES-CTR"))?;
    cipher
        .try_apply_keystream(data)
        .map_err(|_| FormatError::Crypto("AES-CTR"))
}

/// HMAC-SHA256 over the concatenation of `parts`.
pub(super) fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> Result<[u8; 32], FormatError> {
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key)
        .map_err(|_| FormatError::Crypto("HMAC-SHA256"))?;
    for part in parts {
        Mac::update(&mut mac, part);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&mac.finalize().into_bytes());
    Ok(out)
}

/// Constant-time check of an HMAC-SHA256 tag over the concatenation of `parts`.
pub(super) fn hmac_sha256_matches(
    key: &[u8],
    parts: &[&[u8]],
    tag: &[u8],
) -> Result<bool, FormatError> {
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key)
        .map_err(|_| FormatError::Crypto("HMAC-SHA256"))?;
    for part in parts {
        Mac::update(&mut mac, part);
    }
    Ok(mac.verify_slice(tag).is_ok())
}

/// GCM associated data of chunk `index`: chunk number (8 bytes big-endian)
/// ‖ header nonce.
fn gcm_chunk_aad(index: u64, header_nonce: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(8 + header_nonce.len());
    aad.extend_from_slice(&index.to_be_bytes());
    aad.extend_from_slice(header_nonce);
    aad
}

/// Encrypts chunk `index` with a fresh random chunk nonce.
pub(crate) fn encrypt_chunk(
    key: &Masterkey,
    header: &FileHeader,
    index: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, FormatError> {
    let mut nonce = [0u8; CTR_IV];
    let nonce = &mut nonce[..header.combo().nonce_len()];
    fill_random(nonce)?;
    encrypt_chunk_with_nonce(key, header, index, plaintext, nonce)
}

/// Encrypts chunk `index` with the given chunk nonce: nonce ‖ ciphertext ‖
/// tag. Only `encrypt_chunk` and the CryptoLib vectors choose the nonce.
pub(super) fn encrypt_chunk_with_nonce(
    key: &Masterkey,
    header: &FileHeader,
    index: u64,
    plaintext: &[u8],
    nonce: &[u8],
) -> Result<Vec<u8>, FormatError> {
    let combo = header.combo();
    if plaintext.len() > CHUNK_PAYLOAD {
        return Err(FormatError::TooLarge("Block"));
    }
    if nonce.len() != combo.nonce_len() {
        return Err(FormatError::Crypto("Block-Nonce"));
    }
    let mut out = Vec::with_capacity(plaintext.len() + combo.chunk_overhead());
    out.extend_from_slice(nonce);
    out.extend_from_slice(plaintext);
    match combo {
        CipherCombo::SivGcm => {
            let aad = gcm_chunk_aad(index, header.nonce());
            let tag = gcm_seal(header.content_key(), nonce, &aad, &mut out[GCM_NONCE..])?;
            out.extend_from_slice(&tag);
        }
        CipherCombo::SivCtrMac => {
            ctr_apply(header.content_key(), nonce, &mut out[CTR_IV..])?;
            let number = index.to_be_bytes();
            let mac = hmac_sha256(key.mac_key(), &[header.nonce(), &number[..], &out[..]])?;
            out.extend_from_slice(&mac);
        }
    }
    Ok(out)
}

/// Authenticates and decrypts chunk `index` (nonce ‖ ciphertext ‖ tag). The
/// tag (GCM) or the MAC over header nonce ‖ chunk number ‖ nonce ‖
/// ciphertext (CTRMAC, MAC masterkey) is checked before decryption.
pub(crate) fn decrypt_chunk(
    key: &Masterkey,
    header: &FileHeader,
    index: u64,
    chunk: &[u8],
) -> Result<Zeroizing<Vec<u8>>, FormatError> {
    let combo = header.combo();
    if chunk.len() < combo.chunk_overhead() || chunk.len() > combo.chunk_size() {
        return Err(FormatError::InvalidChunk);
    }
    let nonce_len = combo.nonce_len();
    let tag_at = chunk.len() - combo.tag_len();
    let (nonce, tag) = (&chunk[..nonce_len], &chunk[tag_at..]);
    match combo {
        CipherCombo::SivGcm => {
            let mut plain = Zeroizing::new(chunk[nonce_len..tag_at].to_vec());
            let aad = gcm_chunk_aad(index, header.nonce());
            gcm_open(header.content_key(), nonce, &aad, &mut plain, tag, "Block")?;
            Ok(plain)
        }
        CipherCombo::SivCtrMac => {
            let number = index.to_be_bytes();
            let parts: [&[u8]; 3] = [header.nonce(), &number[..], &chunk[..tag_at]];
            if !hmac_sha256_matches(key.mac_key(), &parts, tag)? {
                return Err(FormatError::Authentication("Block"));
            }
            let mut plain = Zeroizing::new(chunk[nonce_len..tag_at].to_vec());
            ctr_apply(header.content_key(), nonce, &mut plain)?;
            Ok(plain)
        }
    }
}
