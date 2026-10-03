//! Size limits of one sync pair, derived from the device's memory instead of
//! fixed caps (V3, Y35/Y67/Y91/Y120). Walks, baselines and the incremental
//! index use the same limits, so a tree a walk accepts can always be stored.

/// Memory one walked entry costs across a run's structures, without its path
/// text: both trees, the previous signatures, the baseline, its planning copy
/// and the new baseline (B-tree nodes, `String` and `Sig` headers, allocator
/// overhead).
const BYTES_PER_ENTRY: u64 = 512;
/// Copies of an entry's path text a run holds at once (trees and baselines).
const TEXT_COPIES: u64 = 4;
/// A run plans with at most a quarter of the physical memory; the rest stays
/// with the system, the transfers and the rest of the app.
const MEMORY_SHARE_DIVISOR: u64 = 4;
/// Framing of one stored baseline entry: length prefix, two presence tags
/// and two signatures (see `persistence.rs`).
const STORED_ENTRY_BYTES: u64 = 4 + 2 + 2 * 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncLimits {
    /// Entries (files, folders, filtered files, omissions) one side's walk
    /// may hold.
    pub walk_entries: u64,
    /// Relative-path bytes one side's walk may hold.
    pub walk_text_bytes: u64,
    /// Entries a stored state (baseline, incremental index) may hold: both
    /// sides' entries.
    pub state_entries: u64,
    /// Relative-path bytes a stored state may hold.
    pub state_text_bytes: u64,
}

impl SyncLimits {
    /// The walk caps before RV1; the state holds both sides. Used when the
    /// physical memory is unknown and as the floor of every derived limit, so
    /// a tree that synchronized before never fails on a smaller device.
    pub const FALLBACK: SyncLimits = SyncLimits {
        walk_entries: 1_000_000,
        walk_text_bytes: 128 * 1024 * 1024,
        state_entries: 2_000_000,
        state_text_bytes: 256 * 1024 * 1024,
    };

    /// Limits for a device with `physical` bytes of memory (`None` = unknown).
    /// Physical rather than currently available memory keeps the limits
    /// stable from run to run.
    pub fn for_memory(physical: Option<u64>) -> SyncLimits {
        let Some(physical) = physical else {
            return Self::FALLBACK;
        };
        // Entries and path text may each use half of the planning share.
        let half = physical / MEMORY_SHARE_DIVISOR / 2;
        let walk_entries = (half / BYTES_PER_ENTRY).max(Self::FALLBACK.walk_entries);
        let walk_text_bytes = (half / TEXT_COPIES).max(Self::FALLBACK.walk_text_bytes);
        SyncLimits {
            walk_entries,
            walk_text_bytes,
            state_entries: walk_entries.saturating_mul(2),
            state_text_bytes: walk_text_bytes.saturating_mul(2),
        }
    }

    /// Bytes a stored baseline within these limits may take.
    pub fn state_file_bytes(&self) -> u64 {
        self.state_entries
            .saturating_mul(STORED_ENTRY_BYTES)
            .saturating_add(self.state_text_bytes)
    }
}
