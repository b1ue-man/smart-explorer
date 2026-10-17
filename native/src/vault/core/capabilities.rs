//! Capabilities of the cleartext view `vault://` (C2.5): never more generous
//! than the storage below it. Most answers pass the inner value through or
//! are fixed (see `VaultBackend`); the pure rules that need arithmetic live
//! here.
use crate::cryptomator::{CipherCombo, CHUNK_PAYLOAD};
use crate::vfs::{StagedWriteCapabilities, TargetLimits};

/// Largest cleartext file whose encrypted form (header, chunks) fits into
/// `stored` bytes of storage.
pub(crate) fn cleartext_capacity(stored: u64, combo: CipherCombo) -> u64 {
    let Some(body) = stored.checked_sub(combo.header_size() as u64) else {
        return 0;
    };
    let chunk = combo.chunk_size() as u64;
    let full = body / chunk;
    let partial = (body % chunk).saturating_sub(combo.chunk_overhead() as u64);
    full.saturating_mul(CHUNK_PAYLOAD as u64)
        .saturating_add(partial)
}

/// What the cleartext view can store, from what the storage below the
/// vault root can store: names are encrypted (and shortened when long), so
/// no naming rule of the storage reaches them and their length stays
/// unknown; file sizes shrink by the encryption overhead; modification
/// times are those of the stored files.
pub(crate) fn cleartext_limits(inner: TargetLimits, combo: CipherCombo) -> TargetLimits {
    TargetLimits {
        windows_names: false,
        max_name: None,
        max_file_size: inner
            .max_file_size
            .map(|stored| cleartext_capacity(stored, combo)),
        mtime_precision: inner.mtime_precision,
    }
}

/// Whether a no-replace publication or rename may target a shortened name
/// (`<digest>.c9s/`): the folder of the long name is the no-replace gate,
/// so the storage must create entries atomically (C2.5: only with inner
/// `create = true`, otherwise `Unsupported`).
pub(crate) fn long_names_no_replace(inner: StagedWriteCapabilities) -> bool {
    inner.create
}
