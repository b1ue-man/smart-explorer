use rayon::prelude::*;

use super::super::platform::is_plain_directory;

/// Stat candidates without following link-like paths, then sort by fuzzy score
/// and modification time. Stale or replaced index entries are omitted.
pub fn stat_and_rank(candidates: Vec<(String, i32)>, max: usize) -> Vec<(String, i32)> {
    let mut with_mtime: Vec<(String, i32, i64)> = candidates
        .into_par_iter()
        .filter_map(|(path, score)| {
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if !is_plain_directory(&metadata) {
                return None;
            }
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_millis()).ok())
                .unwrap_or(0);
            Some((path, score, mtime))
        })
        .collect();
    with_mtime
        .sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| right.2.cmp(&left.2)));
    with_mtime.truncate(max);
    with_mtime
        .into_iter()
        .map(|(path, score, _)| (path, score))
        .collect()
}
