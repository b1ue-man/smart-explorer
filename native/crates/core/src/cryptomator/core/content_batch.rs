//! Many consecutive chunks of one file at once (G4 MT1/MT2): one random
//! draw for all chunk nonces, the chunks encrypted or authenticated and
//! decrypted in parallel, and one contiguous output, so a stream writes
//! about 1 MiB per inner write instead of 32 KiB.
//!
//! The byte format is exactly the per-chunk one: every chunk keeps its own
//! random nonce and its AAD (chunk number ‖ header nonce), chunks stay in
//! order, and every tag or MAC is checked before any plaintext of the batch
//! is released.
use rayon::prelude::*;
use zeroize::Zeroizing;

use super::content_chunks::{decrypt_chunk, encrypt_chunk_with_nonce};
use super::content_header::FileHeader;
use super::keys::{fill_random, Masterkey};
use super::{CipherCombo, FormatError, CHUNK_PAYLOAD};

/// Chunks per batch: 32 × 32 KiB = 1 MiB of cleartext.
pub(super) const BATCH_CHUNKS: usize = 32;

/// Encrypts `plaintext` as the chunks `first`, `first + 1`, … (every chunk
/// but the last one full) and returns their concatenation.
pub(super) fn encrypt_chunks(
    key: &Masterkey,
    header: &FileHeader,
    first: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, FormatError> {
    let combo: CipherCombo = header.combo();
    let count = plaintext.len().div_ceil(CHUNK_PAYLOAD);
    let nonce_len = combo.nonce_len();
    let mut nonces = vec![0u8; count * nonce_len];
    fill_random(&mut nonces)?;
    let seal = |(index, part): (usize, &[u8])| {
        let nonce = &nonces[index * nonce_len..(index + 1) * nonce_len];
        encrypt_chunk_with_nonce(key, header, first + index as u64, part, nonce)
    };
    let chunks: Vec<Vec<u8>> = if count > 1 {
        plaintext
            .par_chunks(CHUNK_PAYLOAD)
            .enumerate()
            .map(seal)
            .collect::<Result<_, _>>()?
    } else {
        plaintext
            .chunks(CHUNK_PAYLOAD)
            .enumerate()
            .map(seal)
            .collect::<Result<_, _>>()?
    };
    let mut out = Vec::with_capacity(plaintext.len() + count * combo.chunk_overhead());
    for chunk in &chunks {
        out.extend_from_slice(chunk);
    }
    Ok(out)
}

/// Authenticates and decrypts the consecutive chunks in `ciphertext`
/// (`first`, `first + 1`, …; the last one may be short). Nothing is
/// returned unless every chunk of the batch verified.
pub(super) fn decrypt_chunks(
    key: &Masterkey,
    header: &FileHeader,
    first: u64,
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, FormatError> {
    let size = header.combo().chunk_size();
    let open =
        |(index, chunk): (usize, &[u8])| decrypt_chunk(key, header, first + index as u64, chunk);
    let plains: Vec<Zeroizing<Vec<u8>>> = if ciphertext.len() > size {
        ciphertext
            .par_chunks(size)
            .enumerate()
            .map(open)
            .collect::<Result<_, _>>()?
    } else {
        ciphertext
            .chunks(size)
            .enumerate()
            .map(open)
            .collect::<Result<_, _>>()?
    };
    let total = plains.iter().map(|plain| plain.len()).sum();
    let mut out = Zeroizing::new(Vec::with_capacity(total));
    for plain in &plains {
        out.extend_from_slice(plain);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "content_batch_tests.rs"]
mod tests;
