//! Lines of a replacement: which matches the selected hits of a search result
//! name, and the lines of each change before and after for a dry run. A hit
//! names a match by its line and its range in the line's text exactly as the
//! search reported it (`line_hit`: the cut of a long line, the decoding of an
//! ANSI line), so the ranges are computed here the same way.
use std::collections::HashMap;

use super::content_lines::{line_hit, without_terminator};
use super::content_replace::{HitRef, ReplacePreview};
use super::content_replace_captures::Found;
use super::content_replace_text::{display, Domain};

/// Share of the text kept before the first change when a long line is cut
/// (as the search keeps a quarter before its first match).
const LEAD_DIVISOR: usize = 4;

/// Bytes the BOM of a UTF-8 file adds to the first line where a reader counts it.
const BOM_BYTES: u32 = 3;

/// Where the lines of a haystack start (split at `\n`, as the search counts).
pub(super) struct Lines {
    starts: Vec<usize>,
    len: usize,
}

impl Lines {
    pub(super) fn new(bytes: &[u8]) -> Self {
        let mut starts = vec![0];
        starts.extend(memchr::memchr_iter(b'\n', bytes).map(|newline| newline + 1));
        Self {
            starts,
            len: bytes.len(),
        }
    }

    /// The 0-based line holding byte `position`.
    fn index(&self, position: usize) -> usize {
        match self.starts.binary_search(&position) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        }
    }

    fn start(&self, index: usize) -> usize {
        self.starts.get(index).copied().unwrap_or(self.len)
    }

    /// The end of line `index` without its terminator.
    fn content_end(&self, bytes: &[u8], index: usize) -> usize {
        let start = self.start(index);
        let end = self.starts.get(index + 1).copied().unwrap_or(self.len);
        start + without_terminator(&bytes[start..end]).len()
    }

    /// First and last line of the match `span`.
    fn span(&self, (start, end): (usize, usize)) -> (usize, usize) {
        let first = self.index(start);
        let last = if end > start {
            self.index(end - 1)
        } else {
            first
        };
        (first, last)
    }
}

/// For every match: whether one of `refs` names it. A hit without ranges
/// names every match touching its line; a match across lines is named by a
/// hit on any of its lines.
pub(super) fn selected(
    bytes: &[u8],
    lines: &Lines,
    found: &[Found],
    refs: &[HitRef],
    bom_shift: bool,
    limit: usize,
) -> Vec<bool> {
    let mut by_line: HashMap<u64, Vec<&HitRef>> = HashMap::new();
    for hit in refs {
        by_line.entry(hit.line).or_default().push(hit);
    }
    let mut chosen = vec![false; found.len()];
    // Per line with hits: the non-empty parts of its matches (line offsets).
    let mut pieces: HashMap<usize, Vec<(usize, usize, usize)>> = HashMap::new();
    for (number, item) in found.iter().enumerate() {
        let (start, end) = item.span();
        let (first, last) = lines.span((start, end));
        for line in first..=last {
            let Some(hits) = by_line.get(&(line as u64 + 1)) else {
                continue;
            };
            if hits.iter().any(|hit| hit.ranges.is_empty()) {
                chosen[number] = true;
            }
            let line_start = lines.start(line);
            let (from, to) = (
                start.max(line_start),
                end.min(lines.content_end(bytes, line)),
            );
            if from < to {
                pieces
                    .entry(line)
                    .or_default()
                    .push((from - line_start, to - line_start, number));
            }
        }
    }
    for (line, parts) in pieces {
        let hits = &by_line[&(line as u64 + 1)];
        let line_start = lines.start(line);
        let text = &bytes[line_start..lines.content_end(bytes, line)];
        let position = (line as u64 + 1, line_start as u64);
        let first = (parts[0].0, parts[0].1);
        let alone = line_hit(position, text, &[first], limit, false).ranges;
        for &(from, to, number) in &parts {
            let shown = if (from, to) == first {
                alone.first().copied()
            } else {
                let both = line_hit(position, text, &[first, (from, to)], limit, false).ranges;
                // The pair keeps one more range exactly when this part is shown.
                if both.len() > alone.len() {
                    both.last().copied()
                } else {
                    None
                }
            };
            let Some(shown) = shown else {
                continue;
            };
            let shifted = (shown.0 + BOM_BYTES, shown.1 + BOM_BYTES);
            let named = hits.iter().any(|hit| {
                hit.ranges
                    .iter()
                    .any(|range| *range == shown || (bom_shift && line == 0 && *range == shifted))
            });
            if named {
                chosen[number] = true;
            }
        }
    }
    chosen
}

/// One preview per group of chosen matches on shared lines. `shifts[k]` is
/// the new text's length minus the old one's after the k-th chosen match.
pub(super) fn previews(
    (before, after): (&[u8], &[u8]),
    domain: Domain,
    lines: &Lines,
    (found, chosen, shifts): (&[Found], &[bool], &[i64]),
    limit: usize,
) -> Vec<ReplacePreview> {
    let spans: Vec<(usize, usize)> = found
        .iter()
        .zip(chosen)
        .filter(|(_, chosen)| **chosen)
        .map(|(item, _)| item.span())
        .collect();
    // (first line, last line, first chosen match, last chosen match, end).
    let mut groups: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
    for (number, span) in spans.iter().enumerate() {
        let (first, last) = lines.span(*span);
        match groups.last_mut() {
            Some(group) if first <= group.1 => {
                group.1 = group.1.max(last);
                group.3 = number;
                group.4 = group.4.max(span.1);
            }
            _ => groups.push((first, last, number, number, span.1)),
        }
    }
    let shift = |position: usize, delta: i64| (position as i64 + delta).max(0) as usize;
    groups
        .into_iter()
        .map(|(first, last, from, to, end)| {
            let start = lines.start(first);
            let stop = lines.content_end(before, last).max(end);
            let lead = if from == 0 { 0 } else { shifts[from - 1] };
            let new_start = shift(start, lead).min(after.len());
            let new_stop = shift(stop, shifts[to]).clamp(new_start, after.len());
            let anchor = spans[from].0 - start;
            let (old, old_cut) = cut(&before[start..stop], anchor, domain, limit);
            let (new, new_cut) = cut(&after[new_start..new_stop], anchor, domain, limit);
            ReplacePreview {
                line: first as u64 + 1,
                before: old,
                after: new,
                truncated: old_cut || new_cut,
            }
        })
        .collect()
}

/// `bytes` as text, cut to `limit` bytes around `anchor` (a quarter before it).
fn cut(bytes: &[u8], anchor: usize, domain: Domain, limit: usize) -> (String, bool) {
    let text = display(bytes, domain);
    let limit = limit.max(1);
    if text.len() <= limit {
        return (text, false);
    }
    let anchor = display(&bytes[..anchor.min(bytes.len())], domain).len();
    let mut start = anchor.saturating_sub(limit / LEAD_DIVISOR);
    if start + limit > text.len() {
        start = text.len() - limit;
    }
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (start + limit).min(text.len());
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[start..end].to_string(), true)
}
