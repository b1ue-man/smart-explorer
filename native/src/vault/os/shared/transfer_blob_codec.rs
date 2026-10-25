//! Authenticated wire schema and bounds for purpose-sealed transfer blobs.
use std::io;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;
use crate::vault::{RootBinding, SealPurpose, MAX_SEALED_TEXT};
use super::invalid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransferBlobPurpose { Job, Alias }
impl TransferBlobPurpose {
    pub(super) fn seal(self) -> SealPurpose { match self { Self::Job => SealPurpose::TransferJob, Self::Alias => SealPurpose::TransferAlias } }
    pub(super) fn wire(self) -> crate::daemon::SealPurposeWire { match self { Self::Job => crate::daemon::SealPurposeWire::TransferJob, Self::Alias => crate::daemon::SealPurposeWire::TransferAlias } }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope { pub(super) schema: u8, pub(super) purpose: TransferBlobPurpose, pub(super) blob: String, pub(super) sites: Vec<String>, pub(super) chunks: Vec<String> }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload { pub(super) schema: u8, pub(super) purpose: TransferBlobPurpose, pub(super) record: String, pub(super) bindings: Vec<RootBinding>, pub(super) contents: String }
impl Drop for Payload { fn drop(&mut self) { self.contents.zeroize(); self.record.zeroize(); } }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Chunk { pub(super) blob: String, pub(super) purpose: TransferBlobPurpose, pub(super) index: usize, pub(super) count: usize, pub(super) bytes: String }
impl Drop for Chunk { fn drop(&mut self) { self.bytes.zeroize(); } }

pub(super) fn chunk_capacity(blob: &str, purpose: TransferBlobPurpose, layers: usize) -> io::Result<usize> {
    if layers == 0 { return Err(invalid()); }
    let empty = Chunk { blob: blob.into(), purpose, index: usize::MAX, count: usize::MAX, bytes: String::new() };
    let overhead = serde_json::to_vec(&empty).map_err(|_| invalid())?.len();
    let fits = |bytes| -> io::Result<bool> {
        let mut length = overhead.checked_add(base64_size(bytes)?).ok_or_else(invalid)?;
        for _ in 0..layers {
            if length > MAX_SEALED_TEXT { return Ok(false); }
            length = sealed_text_size(length)?;
        }
        Ok(true)
    };
    let (mut low, mut high) = (0, MAX_SEALED_TEXT);
    while low < high { let mid = low + (high-low).div_ceil(2); if fits(mid)? { low = mid; } else { high = mid-1; } }
    if low == 0 { Err(invalid()) } else { Ok(low) }
}
pub(super) fn base64_size(size: usize) -> io::Result<usize> { size.checked_mul(4).and_then(|size| size.checked_add(2)).map(|size| size/3).ok_or_else(invalid) }
pub(super) fn sealed_text_size(size: usize) -> io::Result<usize> {
    let ciphertext = crate::cryptomator::file_size(u64::try_from(size).map_err(|_| invalid())?, crate::cryptomator::CipherCombo::SivGcm);
    base64_size(usize::try_from(ciphertext).map_err(|_| invalid())?)
}
