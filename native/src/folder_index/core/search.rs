use rayon::prelude::*;

use super::model::FolderIndex;

impl FolderIndex {
    /// CPU-only part of the search: parallel fuzzy scoring, sorted by score
    /// descending, truncated to `n`. No filesystem access - safe to run on
    /// the UI thread even with a cold disk.
    pub fn search_scored(&self, query: &str, n: usize) -> Vec<(String, i32)> {
        if query.is_empty() || self.paths.is_empty() {
            return Vec::new();
        }
        let q_lower: Vec<u8> = query.bytes().map(|b| b.to_ascii_lowercase()).collect();

        let mut scored: Vec<(String, i32)> = self
            .paths
            .par_iter()
            .filter_map(|p| fuzzy_score(&q_lower, p.as_bytes()).map(|s| (p.clone(), s)))
            .collect();
        scored.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        scored.truncate(n);
        scored
    }
}

/// Subsequence fuzzy scoring. Returns Some(score) if all query chars appear in
/// `target` in order (case-insensitive), or None otherwise.
///
/// Heuristics that make scores feel right for path search:
///   - Consecutive matched chars: +3 each (longer runs = better)
///   - Match right after a separator (`/`, `\`, `_`, `-`, `.`, ` `): +8
///   - Match in basename (after last `/`): +30 total bonus
///   - Match in initial char of target: +12
///   - Gap penalty: -1 per unmatched char
///   - Late matches penalty: lower index = better (subtle)
pub(super) fn fuzzy_score(query: &[u8], target: &[u8]) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    if target.is_empty() {
        return None;
    }

    // Find basename start (after last '/')
    let basename_start = target
        .iter()
        .rposition(|&b| b == b'/' || b == b'\\')
        .map(|i| i + 1)
        .unwrap_or(0);

    let mut score: i32 = 0;
    let mut consecutive: i32 = 0;
    let mut qi = 0;
    let mut last_match_idx: Option<usize> = None;
    let mut matched_in_basename = false;

    for (ti, &tc) in target.iter().enumerate() {
        if qi >= query.len() {
            break;
        }
        let tc_l = tc.to_ascii_lowercase();
        let qc_l = query[qi];

        if tc_l == qc_l {
            score += 4;
            if let Some(last) = last_match_idx {
                if last + 1 == ti {
                    consecutive += 1;
                    score += 3 * consecutive;
                } else {
                    consecutive = 0;
                    // Gap penalty proportional to distance, capped
                    let gap = (ti - last - 1).min(20) as i32;
                    score -= gap;
                }
            } else if ti == 0 {
                score += 12;
            }
            // Word-start bonus
            if ti > 0 && matches!(target[ti - 1], b'/' | b'\\' | b'_' | b'-' | b'.' | b' ') {
                score += 8;
            }
            if ti >= basename_start && !matched_in_basename {
                matched_in_basename = true;
                score += 30;
            }
            last_match_idx = Some(ti);
            qi += 1;
        }
    }

    if qi == query.len() {
        // Bonus for shorter target (more relevant), capped
        let len_penalty = (target.len() as i32 / 8).min(20);
        Some(score - len_penalty)
    } else {
        None
    }
}
