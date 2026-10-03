//! When two observations of one file count as the same (FS2, Y46/Y47/Y52):
//! content hashes where both have one, otherwise size plus modification time
//! at the precision the sides store times with, the job's tolerance and – on
//! FAT, which stores local time – exactly one hour apart after a
//! daylight-saving change.
use crate::vfs::MtimePrecision;

use super::types::{BisyncOptions, CompareMode, PairSide, Sig};

const HOUR_MS: u64 = 3_600_000;

/// The time precision of both sides of a pair (`vfs::mtime_precision`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeRules {
    pub a: MtimePrecision,
    pub b: MtimePrecision,
}

impl TimeRules {
    /// Only identical times are the same (unknown precision on both sides).
    pub const EXACT: TimeRules = TimeRules {
        a: MtimePrecision::Unknown,
        b: MtimePrecision::Unknown,
    };

    /// The precision a comparison between the two sides can rely on.
    pub fn cross(&self) -> MtimePrecision {
        self.a.coarser(self.b)
    }

    pub fn side(&self, side: PairSide) -> MtimePrecision {
        match side {
            PairSide::A => self.a,
            PairSide::B => self.b,
        }
    }
}

/// What is compared: the two sides with each other, or one side with what
/// was recorded for it at the last run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Against {
    OtherSide,
    Baseline,
}

/// Two Unix-millisecond times are the same instant at `precision`, within the
/// job's window, or (FAT) exactly one hour apart. Never overflows (Y52).
pub(super) fn same_time(a_ms: i64, b_ms: i64, precision: MtimePrecision, window_ms: i64) -> bool {
    let distance = a_ms.abs_diff(b_ms);
    let window = u64::try_from(window_ms).unwrap_or(0);
    if distance <= window || precision.same_instant(a_ms, b_ms) {
        return true;
    }
    if !precision.local_time_shifts() {
        return false;
    }
    let shifted = distance.abs_diff(HOUR_MS);
    let step = precision.step_ms().unwrap_or(1).max(1);
    shifted <= step
}

/// Two signatures describe the same file content.
pub(super) fn same_file(
    x: Sig,
    y: Sig,
    precision: MtimePrecision,
    opts: &BisyncOptions,
    _against: Against,
) -> bool {
    if x.size != y.size {
        return false;
    }
    if x.hash != 0 && y.hash != 0 {
        return x.hash == y.hash;
    }
    match opts.compare {
        CompareMode::SizeOnly => true,
        // A checksum comparison cannot certify an unhashed record; the first
        // checksum run upgrades it with authoritative action signatures.
        CompareMode::Checksum => false,
        CompareMode::MtimeSize => {
            same_time(x.mtime_ms, y.mtime_ms, precision, opts.modify_window_ms)
        }
    }
}

/// Presence and content are the same.
pub(super) fn same_entry(
    x: Option<Sig>,
    y: Option<Sig>,
    precision: MtimePrecision,
    opts: &BisyncOptions,
    against: Against,
) -> bool {
    match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => same_file(x, y, precision, opts, against),
        _ => false,
    }
}

/// Both exist with equal size, but neither content nor time decides: reading
/// both could prove them equal (FS1: equal files without a record converge
/// instead of becoming conflicts or copies).
pub(super) fn worth_hashing(x: Option<Sig>, y: Option<Sig>, opts: &BisyncOptions) -> bool {
    match (x, y) {
        (Some(x), Some(y)) => {
            x.size == y.size
                && (x.hash == 0 || y.hash == 0)
                && opts.compare != CompareMode::SizeOnly
        }
        _ => false,
    }
}
