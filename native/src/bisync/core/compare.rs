//! When two observations of one file count as the same (FS2, Y46/Y47/Y52):
//! content hashes where both have one, otherwise size plus modification time
//! at the precision the sides store times with, the job's tolerance and – when
//! either side is FAT, which stores local time – exactly one hour apart after
//! a daylight-saving change (exFAT keeps a UTC offset with its times and is
//! no such side, `MtimePrecision::local_time_shifts`).
use crate::vfs::MtimePrecision;

use super::types::{BisyncOptions, CompareMode, PairSide, Sig};

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

    /// The comparison between the two sides: their common precision, and
    /// the daylight-saving rule when either side stores local time (FAT),
    /// whatever the other side's precision.
    pub fn across(&self) -> TimeCompare {
        TimeCompare {
            precision: self.cross(),
            local_shift: self.a.local_time_shifts() || self.b.local_time_shifts(),
        }
    }

    pub fn side(&self, side: PairSide) -> MtimePrecision {
        match side {
            PairSide::A => self.a,
            PairSide::B => self.b,
        }
    }
}

/// How two times are compared: at `precision`, and with `local_shift` an
/// exact hour apart (within one step) also counts as the same (FAT).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeCompare {
    pub precision: MtimePrecision,
    pub local_shift: bool,
}

impl From<MtimePrecision> for TimeCompare {
    /// One side with its own record: the rule of that side's precision.
    fn from(precision: MtimePrecision) -> Self {
        Self {
            precision,
            local_shift: precision.local_time_shifts(),
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
    same_time_as(a_ms, b_ms, precision.into(), window_ms)
}

/// `same_time` with the daylight-saving rule of `times`: a side with local
/// time (FAT) reports an untouched file exactly one hour off after a clock
/// change (`transfer::time_window::dst_shift_equal`, within one step of the
/// common precision). Sizes are compared before (`same_file`).
pub(super) fn same_time_as(a_ms: i64, b_ms: i64, times: TimeCompare, window_ms: i64) -> bool {
    let distance = a_ms.abs_diff(b_ms);
    let window = u64::try_from(window_ms).unwrap_or(0);
    if distance <= window || times.precision.same_instant(a_ms, b_ms) {
        return true;
    }
    let step = times.precision.step_ms().unwrap_or(1).max(1);
    times.local_shift
        && crate::transfer::time_window::dst_shift_equal(
            a_ms,
            b_ms,
            i64::try_from(step).unwrap_or(i64::MAX),
        )
}

/// Two signatures describe the same file content.
pub(super) fn same_file(
    x: Sig,
    y: Sig,
    times: impl Into<TimeCompare>,
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
            same_time_as(x.mtime_ms, y.mtime_ms, times.into(), opts.modify_window_ms)
        }
    }
}

/// Presence and content are the same.
pub(super) fn same_entry(
    x: Option<Sig>,
    y: Option<Sig>,
    times: impl Into<TimeCompare>,
    opts: &BisyncOptions,
    against: Against,
) -> bool {
    match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => same_file(x, y, times, opts, against),
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

#[cfg(test)]
#[path = "abgleich_task_compare_tests.rs"]
mod abgleich_task_tests;
