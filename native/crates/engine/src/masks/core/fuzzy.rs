//! Fuzzy entries: the two-budget rule of plan S2 (reference
//! docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md §2.4).
//!
//! A candidate is a run of whole word tokens of the text joined only by
//! separators (so a match never borrows letters of a neighbouring word,
//! §2.2). It matches the folded pattern P (length L) when for some cut of
//! `a` characters at the start and `b` at the end of P with `a + b ≤ trim`
//! the weighted distance between the core `P[a..L-b]` and the folded
//! candidate is at most the inner budget of the core. Costs in tenths:
//! insert, delete, substitute and adjacent transposition 10, an OCR pair 4.
//!
//! Exact folded occurrences of the pattern are found separately (also
//! inside compounds) by `matcher` with Aho-Corasick. The plan's piece filter
//! (k+1 pieces) does not separate here: with L = 14, trim 4 and up to five
//! OCR operations it would need 11 pieces from the six characters every
//! core keeps; candidates are therefore bounded by length, by a character
//! filter and by a banded cut-off of the DP instead.
use super::fold::{extra_gap_chars, fold_pattern, is_base_separator, Token, MAX_GAP_CHARS};
use super::model::Tolerance;

/// Insert, delete, substitute, transpose.
const EDIT: u32 = 10;
/// One OCR confusion (plan S2: rn↔m, 0↔O, 1↔l↔I, ß↔B, e↔c, h↔b).
const OCR: u32 = 4;
const INF: u32 = u32::MAX / 4;

/// Folded OCR confusions; each pair applies in both directions.
const OCR_PAIRS: &[(&str, &str)] = &[
    ("rn", "m"),
    ("0", "o"),
    ("1", "l"),
    ("l", "i"),
    ("1", "i"),
    ("ss", "b"),
    ("e", "c"),
    ("h", "b"),
];

/// A compiled fuzzy entry.
#[derive(Clone, Debug)]
pub(crate) struct FuzzyPattern {
    pattern: Vec<char>,
    tolerance: Tolerance,
    extra_gap: Vec<char>,
    allowed: Vec<char>,
    trim: usize,
    max_budget: u32,
    min_len: usize,
    max_len: usize,
    /// Per end position `i` of P (1-based): OCR pairs whose pattern side
    /// ends there, as (pattern side length, text side).
    ocr_at: Vec<Vec<(usize, Vec<char>)>>,
}

/// A match in byte offsets of the text with its inner cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FuzzyHit {
    pub start: usize,
    pub end: usize,
    pub cost: u32,
}

impl FuzzyPattern {
    /// `None` when the value has no word character.
    pub(crate) fn new(value: &str, tolerance: Tolerance) -> Option<Self> {
        let pattern = fold_pattern(value);
        if pattern.is_empty() {
            return None;
        }
        let len = pattern.len();
        let trim = tolerance.trim(len);
        let max_budget = (1..=len)
            .map(|core| tolerance.inner_budget(core))
            .max()
            .unwrap_or(0);
        let slack = (max_budget / OCR) as usize;
        let mut ocr_at = vec![Vec::new(); len + 1];
        let mut allowed = pattern.clone();
        for (left, right) in OCR_PAIRS {
            for (from, to) in [(*left, *right), (*right, *left)] {
                let from: Vec<char> = from.chars().collect();
                let to: Vec<char> = to.chars().collect();
                if from.iter().all(|c| pattern.contains(c)) {
                    allowed.extend(to.iter().copied());
                }
                for end in from.len()..=len {
                    if pattern[end - from.len()..end] == from[..] {
                        ocr_at[end].push((from.len(), to.clone()));
                    }
                }
            }
        }
        allowed.sort_unstable();
        allowed.dedup();
        Some(Self {
            min_len: len.saturating_sub(trim + slack).max(1),
            max_len: len + slack,
            pattern,
            tolerance,
            extra_gap: extra_gap_chars(value),
            allowed,
            trim,
            max_budget,
            ocr_at,
        })
    }

    pub(crate) fn folded(&self) -> String {
        self.pattern.iter().collect()
    }

    /// Longest candidate in folded characters.
    pub(crate) fn max_len(&self) -> usize {
        self.max_len
    }

    /// Matches among the word runs of `text` (`tokens` from
    /// `fold::tokens(text, base)`); `text` starts at byte `base`. The result
    /// holds no overlapping hits: cheaper and then longer hits win.
    pub(crate) fn find(&self, text: &str, base: usize, tokens: &[Token]) -> Vec<FuzzyHit> {
        let mut found = Vec::new();
        let mut candidate: Vec<char> = Vec::new();
        for first in 0..tokens.len() {
            candidate.clear();
            let mut best: Option<FuzzyHit> = None;
            for last in first..tokens.len() {
                if last > first {
                    let gap = &text[tokens[last - 1].end - base..tokens[last].start - base];
                    if !self.gap_allowed(gap) {
                        break;
                    }
                }
                candidate.extend_from_slice(&tokens[last].folded);
                if candidate.len() > self.max_len {
                    break;
                }
                if candidate.len() < self.min_len {
                    continue;
                }
                if let Some(cost) = self.judge(&candidate) {
                    let hit = FuzzyHit {
                        start: tokens[first].start,
                        end: tokens[last].end,
                        cost,
                    };
                    if best.is_none_or(|b| cost < b.cost || (cost == b.cost && hit.end > b.end)) {
                        best = Some(hit);
                    }
                }
            }
            found.extend(best);
        }
        select_disjoint(found)
    }

    fn gap_allowed(&self, gap: &str) -> bool {
        let mut count = 0;
        for c in gap.chars() {
            count += 1;
            if count > MAX_GAP_CHARS || !(is_base_separator(c) || self.extra_gap.contains(&c)) {
                return false;
            }
        }
        count > 0
    }

    /// The smallest inner cost of the folded candidate under the two-budget
    /// rule, or `None`.
    pub(crate) fn judge(&self, candidate: &[char]) -> Option<u32> {
        let foreign = candidate
            .iter()
            .filter(|c| self.allowed.binary_search(c).is_err())
            .count() as u32;
        if foreign.saturating_mul(EDIT) > self.max_budget {
            return None;
        }
        let len = self.pattern.len();
        let mut best: Option<u32> = None;
        for cut_front in 0..=self.trim {
            let ends = self.core_costs(cut_front, candidate);
            for cut_back in 0..=self.trim - cut_front {
                let Some(core) = len.checked_sub(cut_front + cut_back).filter(|c| *c > 0) else {
                    continue;
                };
                let Some(&cost) = ends.get(core) else {
                    continue;
                };
                if cost <= self.tolerance.inner_budget(core) && best.is_none_or(|b| cost < b) {
                    best = Some(cost);
                }
            }
        }
        best
    }

    /// Weighted global distance between every prefix `P[a..a+i]` and the
    /// whole candidate: index `i` of the result. Rows stop once two rows in
    /// a row exceed the largest budget (no later row can come back below it).
    fn core_costs(&self, a: usize, text: &[char]) -> Vec<u32> {
        let pattern = &self.pattern[a..];
        let m = text.len();
        let mut rows: Vec<Vec<u32>> = vec![(0..=m as u32).map(|j| j * EDIT).collect()];
        let mut ends = vec![m as u32 * EDIT];
        for i in 1..=pattern.len() {
            let mut row = vec![INF; m + 1];
            row[0] = i as u32 * EDIT;
            for j in 1..=m {
                let up = &rows[i - 1];
                let mut cost = (up[j] + EDIT).min(row[j - 1] + EDIT);
                let same = pattern[i - 1] == text[j - 1];
                cost = cost.min(up[j - 1] + if same { 0 } else { EDIT });
                if i > 1 && j > 1 && pattern[i - 1] == text[j - 2] && pattern[i - 2] == text[j - 1]
                {
                    cost = cost.min(rows[i - 2][j - 2] + EDIT);
                }
                for (from_len, to) in &self.ocr_at[a + i] {
                    if *from_len > i || to.len() > j || text[j - to.len()..j] != to[..] {
                        continue;
                    }
                    cost = cost.min(rows[i - from_len][j - to.len()] + OCR);
                }
                row[j] = cost;
            }
            ends.push(row[m]);
            let row_min = row.iter().copied().min().unwrap_or(INF);
            let previous_min = rows[i - 1].iter().copied().min().unwrap_or(INF);
            rows.push(row);
            if row_min > self.max_budget && previous_min > self.max_budget {
                break;
            }
        }
        ends
    }
}

/// Non-overlapping hits: cheapest first, then longest, then earliest.
fn select_disjoint(mut hits: Vec<FuzzyHit>) -> Vec<FuzzyHit> {
    hits.sort_by(|x, y| {
        x.cost
            .cmp(&y.cost)
            .then((y.end - y.start).cmp(&(x.end - x.start)))
            .then(x.start.cmp(&y.start))
    });
    let mut chosen: Vec<FuzzyHit> = Vec::new();
    for hit in hits {
        if chosen
            .iter()
            .all(|c| hit.end <= c.start || hit.start >= c.end)
        {
            chosen.push(hit);
        }
    }
    chosen.sort_by_key(|hit| hit.start);
    chosen
}
