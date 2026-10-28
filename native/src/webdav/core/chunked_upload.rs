//! The rules of Nextcloud's chunked upload (G3-12, 2026-10-28), as pure
//! functions: where a connection's upload folders live, which part numbers
//! the ranges of a segmented stage get, and whether this server was seen to
//! take chunked uploads.
//!
//! Nextcloud (developer manual "Chunked file upload", fetched 2026-10-28):
//! `MKCOL /remote.php/dav/uploads/<user>/<id>` with `Destination`, then one
//! `PUT .../<id>/<n>` per part (`n` from 1 to 10000, parts 5 MB to 5 GB except
//! the last, assembled in the order of their names) with `Destination` and
//! `OC-Total-Length`, then `MOVE .../<id>/.file` to the destination; `DELETE`
//! of the folder aborts. Parts are named with five digits (`00001`), as the
//! manual's examples, so name order and number order agree.
//!
//! A part is numbered by its offset (`offset / unit + 1`). The engine's
//! ranges are at least 8 MiB long (`transfer/core/segments.rs`
//! `MIN_SEGMENT`), so with `unit` ≤ 8 MiB two ranges never share a number
//! and the numbers follow the offsets. A file whose unit would exceed 8 MiB
//! (more than 10000 × 8 MiB, about 78 GiB) is not chunked.
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// Smallest part except the last (manual: 5 MB; the binary reading is the
/// larger one).
pub(super) const MIN_PART: u64 = 5 * 1024 * 1024;
/// Largest part (manual: 5 GB; the decimal reading is the smaller one).
pub(super) const MAX_PART: u64 = 5_000_000_000;
/// Highest part number.
pub(super) const MAX_PARTS: u64 = 10_000;
/// The engine's shortest range (`transfer/core/segments.rs` `MIN_SEGMENT`).
const ENGINE_MIN_RANGE: u64 = 8 * 1024 * 1024;
/// Remembered answers about chunked uploads; beyond this the map starts over.
const MAX_REMEMBERED: usize = 1024;

/// The upload folder collection of the account whose files `path` lies in:
/// `<prefix>/remote.php/dav/uploads/<user>` for a path below
/// `<prefix>/remote.php/dav/files/<user>/`; `None` for any other server or
/// form (the legacy `/remote.php/webdav` keeps the single `PUT`).
pub(super) fn upload_home(path: &str) -> Option<String> {
    const FILES: &str = "/remote.php/dav/files/";
    let at = path.find(FILES)?;
    let rest = &path[at + FILES.len()..];
    let (user, below) = rest.split_once('/')?;
    if user.is_empty() || below.is_empty() {
        return None;
    }
    Some(format!("{}/remote.php/dav/uploads/{user}", &path[..at]))
}

/// Bytes per part number for a file of `size` bytes; `None` when the file
/// cannot be numbered within 10000 parts at the engine's range length.
pub(super) fn unit(size: u64) -> Option<u64> {
    let unit = MIN_PART.max(size.div_ceil(MAX_PARTS));
    (unit <= ENGINE_MIN_RANGE).then_some(unit)
}

/// The number of the part starting at `offset`.
pub(super) fn part_number(offset: u64, unit: u64) -> u64 {
    offset / unit.max(1) + 1
}

/// The name of part `number` (five digits).
pub(super) fn part_name(number: u64) -> String {
    format!("{number:05}")
}

/// One range as parts of at most `MAX_PART` bytes, nearly equal in length:
/// `(offset, len)` each, together exactly the range.
pub(super) fn pieces(offset: u64, len: u64) -> Vec<(u64, u64)> {
    let count = len.div_ceil(MAX_PART).max(1);
    let base = len / count;
    let mut pieces = Vec::with_capacity(count as usize);
    let mut start = offset;
    for index in 0..count {
        let piece = if index + 1 == count {
            offset + len - start
        } else {
            base
        };
        pieces.push((start, piece));
        start += piece;
    }
    pieces
}

fn remembered() -> MutexGuard<'static, HashMap<String, bool>> {
    static SUPPORT: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    SUPPORT
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Whether the server behind `key` took (`true`) or refused (`false`) an
/// upload folder; `None` = not asked yet.
pub(super) fn support(key: &str) -> Option<bool> {
    remembered().get(key).copied()
}

pub(super) fn remember_support(key: &str, supported: bool) {
    let mut remembered = remembered();
    if remembered.len() >= MAX_REMEMBERED {
        remembered.clear();
    }
    remembered.insert(key.to_string(), supported);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn speed_task_webdav_chunk_upload_home_only_for_nextcloud_dav_paths() {
        assert_eq!(
            upload_home("/remote.php/dav/files/alice/Docs/a.bin").as_deref(),
            Some("/remote.php/dav/uploads/alice")
        );
        assert_eq!(
            upload_home("/cloud/remote.php/dav/files/bob/a.bin").as_deref(),
            Some("/cloud/remote.php/dav/uploads/bob")
        );
        assert_eq!(upload_home("/remote.php/webdav/a.bin"), None);
        assert_eq!(upload_home("/dav/a.bin"), None);
        assert_eq!(upload_home("/remote.php/dav/files/alice"), None);
    }

    #[test]
    fn speed_task_webdav_chunk_numbers_follow_offsets_and_stay_in_range() {
        assert_eq!(unit(20 * MIB), Some(MIN_PART));
        assert_eq!(unit(10_000 * 8 * MIB), Some(8 * MIB));
        assert_eq!(unit(10_000 * 8 * MIB + 1), None);
        // Ranges of at least 8 MiB never share a number.
        for size in [16 * MIB, 48 * 1024 * MIB, 10_000 * 8 * MIB] {
            let unit = unit(size).unwrap();
            let mut last = 0;
            let mut offset = 0;
            while offset < size {
                let number = part_number(offset, unit);
                assert!(number > last && number <= MAX_PARTS, "{size} {offset}");
                last = number;
                offset += 8 * MIB;
            }
        }
        assert_eq!(part_name(1), "00001");
        assert_eq!(part_name(10_000), "10000");
    }

    #[test]
    fn speed_task_webdav_chunk_pieces_cover_the_range_within_limits() {
        assert_eq!(pieces(8 * MIB, 8 * MIB), vec![(8 * MIB, 8 * MIB)]);
        let len = 12_000_000_001;
        let parts = pieces(5, len);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts.iter().map(|part| part.1).sum::<u64>(), len);
        assert_eq!(parts[0].0, 5);
        for window in parts.windows(2) {
            assert_eq!(window[0].0 + window[0].1, window[1].0);
        }
        let within = |part: &(u64, u64)| part.1 <= MAX_PART && part.1 >= MIN_PART;
        assert!(parts.iter().all(within));
    }
}
