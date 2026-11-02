//! Wire layout of a walk's scope (`walk-filter-v1`, `walk_filter.rs`) in
//! `WalkHashed3`, `WalkKeyed` and `ListRuns`: two bits of the request's
//! option byte (`SCOPE_FRESH`, `SCOPE_FILTER`), and after the request's
//! own fields the filter when `SCOPE_FILTER` is set: a flag byte
//! (`FILTER_*`), the bounds (`u64` minimum and maximum size, `i64` earliest
//! and latest time) and the patterns (`u32` count, then each as a string).
//! A server without the label rejects the bits, so a client sets them only
//! after the label.
use std::io;

use super::super::super::walk_filter::{
    WalkFilterSpec, WalkScope, MAX_FILTER_PATTERNS, MAX_FILTER_TEXT,
};
use super::super::{bad, Reader};
use super::{add_len, put_i64, put_str, put_u32, put_u64};

/// Option bit: digests are read from the files now.
pub(super) const SCOPE_FRESH: u8 = 2;
/// Option bit: the filter follows the request's fields.
pub(super) const SCOPE_FILTER: u8 = 4;
/// Both scope bits.
pub(super) const SCOPE_BITS: u8 = SCOPE_FRESH | SCOPE_FILTER;
const FILTER_HIDDEN: u8 = 1;
const FILTER_BACKSLASH: u8 = 2;

/// The option bits of `scope`.
pub(super) fn bits(scope: &WalkScope) -> u8 {
    let mut bits = 0;
    if scope.fresh {
        bits |= SCOPE_FRESH;
    }
    if scope.filter.is_some() {
        bits |= SCOPE_FILTER;
    }
    bits
}

/// Encoded length of the filter of `scope` (0 without one).
pub(super) fn len(scope: &WalkScope) -> io::Result<usize> {
    let Some(filter) = &scope.filter else {
        return Ok(0);
    };
    if filter.patterns.len() > MAX_FILTER_PATTERNS {
        return Err(bad("walk filter has too many patterns"));
    }
    let mut length = 1 + 32 + 4;
    for pattern in &filter.patterns {
        add_len(&mut length, super::string_len(pattern)?)?;
    }
    Ok(length)
}

/// Appends the filter of `scope`, if any.
pub(super) fn put(b: &mut Vec<u8>, scope: &WalkScope) {
    let Some(filter) = &scope.filter else {
        return;
    };
    let mut flags = 0;
    if filter.skip_hidden {
        flags |= FILTER_HIDDEN;
    }
    if filter.backslash_separates {
        flags |= FILTER_BACKSLASH;
    }
    b.push(flags);
    put_u64(b, filter.min_size);
    put_u64(b, filter.max_size);
    put_i64(b, filter.after_mtime_ms);
    put_i64(b, filter.before_mtime_ms);
    put_u32(b, filter.patterns.len() as u32);
    for pattern in &filter.patterns {
        put_str(b, pattern);
    }
}

/// The scope of a request whose option byte carried `bits`.
pub(super) fn take(bits: u8, r: &mut Reader) -> io::Result<WalkScope> {
    let filter = if bits & SCOPE_FILTER != 0 {
        let flags = r.u8()?;
        if flags & !(FILTER_HIDDEN | FILTER_BACKSLASH) != 0 {
            return Err(bad("unknown walk filter flags"));
        }
        let min_size = r.u64()?;
        let max_size = r.u64()?;
        let after_mtime_ms = r.i64()?;
        let before_mtime_ms = r.i64()?;
        let count = r.u32()? as usize;
        if count > MAX_FILTER_PATTERNS || count > r.remaining() / 4 {
            return Err(bad("walk filter has too many patterns"));
        }
        let mut patterns = Vec::with_capacity(count);
        let mut text = 0usize;
        for _ in 0..count {
            let pattern = r.string()?;
            text = text.saturating_add(pattern.len());
            if text > MAX_FILTER_TEXT {
                return Err(bad("walk filter patterns too long"));
            }
            patterns.push(pattern);
        }
        Some(WalkFilterSpec {
            skip_hidden: flags & FILTER_HIDDEN != 0,
            patterns,
            backslash_separates: flags & FILTER_BACKSLASH != 0,
            min_size,
            max_size,
            after_mtime_ms,
            before_mtime_ms,
        })
    } else {
        None
    };
    Ok(WalkScope {
        fresh: bits & SCOPE_FRESH != 0,
        filter,
    })
}
