//! The daylight-saving rule for filesystems that store local time (FAT:
//! `MtimePrecision::local_time_shifts`). Such a volume reports every time
//! shifted by exactly one hour after a clock change, so a file that was not
//! touched appears one hour newer or older than its copy on a UTC-based side.
//! Callers use it only when a side keeps local time (`fs_profile`), only
//! together with equal sizes, and only when the contents are proven equal by
//! digests each side forms where it stores the file (`hour_proof.rs`):
//! without that proof an hour newer is newer. It decides nothing for any
//! other pair.

#[path = "hour_proof.rs"]
mod hour_proof;
pub use hour_proof::{same_content_where_stored, Stored};

/// One hour in milliseconds.
pub const HOUR_MS: i64 = 3_600_000;

/// Whether `a_ms` and `b_ms` (Unix milliseconds) differ by exactly one hour
/// (3600 s), give or take `window_ms` (the coarser time resolution of the two
/// sides). A distance of zero is not a shift; the ordinary window rule
/// handles it. Never overflows for any pair of times.
pub fn dst_shift_equal(a_ms: i64, b_ms: i64, window_ms: i64) -> bool {
    let window = u64::try_from(window_ms).unwrap_or(0);
    let distance = a_ms.abs_diff(b_ms);
    distance.abs_diff(HOUR_MS.unsigned_abs()) <= window
}

#[cfg(test)]
#[path = "time_window_tests.rs"]
mod tests;
