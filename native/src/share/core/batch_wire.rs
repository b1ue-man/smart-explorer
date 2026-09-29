//! Transfer v1 of the Share filesystem protocol: what a host advertises,
//! the batch requests and their outcomes, and the split of a batch by the
//! encoded size of its header (K18a).
use std::io;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use super::FsErrorKind;
use crate::share::keepalive::TRANSFER_STREAMS_PER_CONNECTION;

/// Hello capability of clients that understand a `Busy` reply: the host then
/// answers them at once when it is full instead of queueing their transfers
/// while the client's deadline runs.
pub(crate) const TRANSFER_V1_CAPABILITY: &str = "fs_transfer_v1";

/// Files per batch (research §3.1): an ordinary entry (path and size, well
/// below 1 KiB encoded) keeps a full header inside the host's 256 KiB first
/// frame, and one commit reports at most this many outcomes.
pub(crate) const BATCH_MAX_FILES: u32 = 256;

/// Bytes per batch (research §3.1): one stream's receive window (16 MiB), so
/// a whole batch is in flight at once; at 100 Mbit/s it holds its admission
/// slot for about 1.3 s.
pub(crate) const BATCH_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// A batch header is the first frame of its stream; the host reads it with
/// its 256 KiB request limit, one byte of which is the frame tag.
pub(crate) const MAX_BATCH_HEADER_BYTES: usize = crate::share::framing::MAX_REQUEST_CTRL_FRAME - 1;

/// Length of the client nonce in hex digits (64 random bits): unique among
/// the stages of one folder, and the stage suffix stays about as short as the
/// existing `.se-peer-…` names.
pub(crate) const NONCE_HEX_LEN: usize = 16;

/// Longest nonce a host accepts (256 bits); later clients may send longer
/// ones, and stage names stay far below the 255-byte name limit.
const MAX_NONCE_HEX_LEN: usize = 64;

/// Transfer features and limits of a host, carried inside the Capabilities
/// reply. All zero (absent on the wire) means a host before transfer v1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsTransferCapabilities {
    /// PutBatch, PutBatchStatus, GetBatch, ReadAt, CreateDir,
    /// PromoteNoReplace and DiscardStage are understood.
    #[serde(default)]
    pub(crate) v1: bool,
    /// Transfers one connection may run at once without a `Busy` reply.
    #[serde(default)]
    pub(crate) admission: u32,
    #[serde(default)]
    pub(crate) batch_max_files: u32,
    #[serde(default)]
    pub(crate) batch_max_bytes: u64,
}

impl FsTransferCapabilities {
    /// What this build offers as a host.
    pub(crate) fn host() -> Self {
        Self {
            v1: true,
            admission: TRANSFER_STREAMS_PER_CONNECTION,
            batch_max_files: BATCH_MAX_FILES,
            batch_max_bytes: BATCH_MAX_BYTES,
        }
    }

    pub(crate) fn is_absent(&self) -> bool {
        *self == Self::default()
    }
}

/// One new file of a PutBatch; its bytes follow in request order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsBatchPut {
    pub(crate) path: String,
    pub(crate) size: u64,
}

/// One file of a GetBatch; `id` addresses one of several equal names on ID
/// providers exported by the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsBatchGet {
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    pub(crate) size: u64,
}

/// Result of one PutBatch entry: the published path or why it failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "o", rename_all = "snake_case")]
pub(crate) enum FsBatchOutcome {
    Published {
        path: String,
    },
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<FsErrorKind>,
        msg: String,
    },
}

impl FsBatchOutcome {
    pub(crate) fn failed(error: &io::Error) -> Self {
        Self::Failed {
            kind: crate::share::fs_error::kind_of(error),
            msg: error.to_string(),
        }
    }
}

/// State of a PutBatch as the host knows it: its reply, or the answer to a
/// status query after the reply was lost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "s", rename_all = "snake_case")]
pub(crate) enum FsBatchStatus {
    /// Still receiving or publishing.
    Pending,
    /// Ended before its commit; nothing was published.
    Aborted,
    /// Committed; one outcome per entry in request order.
    Done { outcomes: Vec<FsBatchOutcome> },
}

impl FsBatchStatus {
    /// One line for logs and error messages.
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Pending => "Paket läuft noch".into(),
            Self::Aborted => "Paket abgebrochen".into(),
            Self::Done { outcomes } => {
                let failed = outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, FsBatchOutcome::Failed { .. }))
                    .count();
                let published = outcomes.len() - failed;
                format!("Paket: {published} veröffentlicht, {failed} fehlgeschlagen")
            }
        }
    }
}

/// Marker of the transfer engine's private upload stages: the engine names
/// them `<file>.se-upload-<16 hex>` (`stage_name`), and its commit helper
/// gets the same shape from `vfs::unique_staging_path(…, "upload")`.
const UPLOAD_STAGE_MARKER: &str = ".se-upload-";

/// Hex digits of a stage suffix (`{:016x}` of 64 random bits).
const STAGE_SUFFIX_HEX_LEN: usize = 16;

/// Whether a client may have the host discard the file `name` (K17): only
/// the engine's upload stages. The host's own batch (`.se-batch-`) and
/// replace (`.se-peer-`) stages, other purposes and every user file are
/// refused.
pub(crate) fn discardable_stage(name: &str) -> bool {
    let Some(position) = name.rfind(UPLOAD_STAGE_MARKER) else {
        return false;
    };
    let suffix = name.get(position + UPLOAD_STAGE_MARKER.len()..);
    position > 0
        && suffix.is_some_and(|suffix| {
            suffix.len() == STAGE_SUFFIX_HEX_LEN
                && suffix
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

/// A client nonce is lowercase hex; it becomes part of stage file names.
pub(crate) fn valid_nonce(nonce: &str) -> bool {
    (NONCE_HEX_LEN..=MAX_NONCE_HEX_LEN).contains(&nonce.len())
        && nonce
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Host-side bounds of a PutBatch before it takes an admission slot.
pub(crate) fn validate_put(nonce: &str, entries: &[FsBatchPut]) -> io::Result<()> {
    if !valid_nonce(nonce) {
        return Err(invalid("Ungültige Paket-Kennung".into()));
    }
    validate_bounds(entries.len(), entries.iter().map(|entry| entry.size))
}

/// Host-side bounds of a GetBatch before it takes an admission slot.
pub(crate) fn validate_get(items: &[FsBatchGet]) -> io::Result<()> {
    validate_bounds(items.len(), items.iter().map(|item| item.size))
}

fn validate_bounds(count: usize, sizes: impl Iterator<Item = u64>) -> io::Result<()> {
    if count == 0 || count > BATCH_MAX_FILES as usize {
        return Err(invalid(format!(
            "Ein Paket enthält 1 bis {BATCH_MAX_FILES} Dateien"
        )));
    }
    let mut total = 0u64;
    for size in sizes {
        total = total
            .checked_add(size)
            .filter(|total| *total <= BATCH_MAX_BYTES)
            .ok_or_else(|| {
                invalid(format!(
                    "Ein Paket umfasst höchstens {BATCH_MAX_BYTES} Bytes"
                ))
            })?;
    }
    Ok(())
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// One request of a split batch: consecutive items, or one item that fits
/// no batch (larger than the byte limit, or a header of its own too large).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BatchPart {
    Items(Range<usize>),
    Oversized(usize),
}

/// Splits `items` (encoded JSON length, byte size) into requests by count,
/// bytes and encoded header size: `envelope` is the encoded request with an
/// empty list, and every item adds its length plus one separator.
pub(crate) fn plan_batches(
    envelope: usize,
    items: &[(usize, u64)],
    max_files: usize,
    max_bytes: u64,
) -> Vec<BatchPart> {
    let max_files = max_files.max(1);
    let mut parts = Vec::new();
    let mut start = 0;
    let mut header = envelope;
    let mut bytes = 0u64;
    for (index, &(encoded, size)) in items.iter().enumerate() {
        let item_header = encoded.saturating_add(1);
        if size > max_bytes || envelope.saturating_add(item_header) > MAX_BATCH_HEADER_BYTES {
            if start < index {
                parts.push(BatchPart::Items(start..index));
            }
            parts.push(BatchPart::Oversized(index));
            start = index + 1;
            header = envelope;
            bytes = 0;
            continue;
        }
        let full = index - start == max_files
            || bytes.saturating_add(size) > max_bytes
            || header.saturating_add(item_header) > MAX_BATCH_HEADER_BYTES;
        if full {
            parts.push(BatchPart::Items(start..index));
            start = index;
            header = envelope;
            bytes = 0;
        }
        header += item_header;
        bytes += size;
    }
    if start < items.len() {
        parts.push(BatchPart::Items(start..items.len()));
    }
    parts
}

#[cfg(test)]
#[path = "batch_wire_tests.rs"]
mod tests;
