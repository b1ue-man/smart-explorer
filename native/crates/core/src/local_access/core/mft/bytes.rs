//! Bounds-checked little-endian reads: a short or truncated slice yields
//! `None`, never a panic.

pub(super) fn u8_at(bytes: &[u8], at: usize) -> Option<u8> {
    bytes.get(at).copied()
}

pub(super) fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

pub(super) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

pub(super) fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    let slice = bytes.get(at..at.checked_add(8)?)?;
    Some(u64::from_le_bytes(slice.try_into().ok()?))
}

pub(super) fn i64_at(bytes: &[u8], at: usize) -> Option<i64> {
    u64_at(bytes, at).map(|value| value as i64)
}

/// `count` UTF-16 code units starting at `at`.
pub(super) fn utf16_at(bytes: &[u8], at: usize, count: usize) -> Option<Vec<u16>> {
    let end = at.checked_add(count.checked_mul(2)?)?;
    let slice = bytes.get(at..end)?;
    Some(
        slice
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect(),
    )
}
