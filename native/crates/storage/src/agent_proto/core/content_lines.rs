//! One line of a searched file as a hit carries it: the text around the first
//! match, cut to the line length of the query, decoded for display (UTF-8, or
//! Windows-1252 for a line that is not valid UTF-8, as Notepad++ shows an ANSI
//! file), with the match ranges moved into the decoded text.
use super::content_query::LineHit;

/// Longest line text a hit carries whatever the query asks for, so a batch of
/// hits always fits a protocol frame (64 MiB, with one range per byte).
pub(super) const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Bytes kept before the first match of a cut line: a quarter of the limit.
const LEAD_DIVISOR: usize = 4;

/// The part of a line that ends at its terminator (`\n`, `\r\n`), without it.
pub(super) fn without_terminator(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// The hit (or context line) of `line` (without terminator): `ranges` are the
/// byte ranges of the matches within `line`, ascending and disjoint; `limit`
/// is the longest text in bytes.
pub(super) fn line_hit(
    (number, offset): (u64, u64),
    line: &[u8],
    ranges: &[(usize, usize)],
    limit: usize,
    context: bool,
) -> LineHit {
    let limit = limit.clamp(1, MAX_LINE_BYTES);
    let utf8 = std::str::from_utf8(line).is_ok();
    let (start, end) = window(line, ranges.first().map_or(0, |range| range.0), limit, utf8);
    let mut text = String::with_capacity(end - start + 8);
    let mut kept = Vec::with_capacity(ranges.len());
    let mut position = start;
    for &(from, to) in ranges {
        let (from, to) = (from.clamp(start, end), to.clamp(start, end));
        if to <= from || from < position {
            continue;
        }
        decode_into(&mut text, &line[position..from], utf8);
        let begin = text.len();
        decode_into(&mut text, &line[from..to], utf8);
        kept.push((begin as u32, text.len() as u32));
        position = to;
    }
    decode_into(&mut text, &line[position..end], utf8);
    LineHit {
        line: number,
        offset,
        text,
        ranges: kept,
        truncated: start > 0 || end < line.len(),
        context,
    }
}

/// `[start, end)` of at most `limit` bytes around `anchor`, on character
/// boundaries of a UTF-8 line.
fn window(line: &[u8], anchor: usize, limit: usize, utf8: bool) -> (usize, usize) {
    if line.len() <= limit {
        return (0, line.len());
    }
    let mut start = anchor.saturating_sub(limit / LEAD_DIVISOR);
    if start + limit > line.len() {
        start = line.len() - limit;
    }
    let mut end = start + limit;
    if utf8 {
        while start < end && is_continuation(line[start]) {
            start += 1;
        }
        while end > start && end < line.len() && is_continuation(line[end]) {
            end -= 1;
        }
    }
    (start, end)
}

fn is_continuation(byte: u8) -> bool {
    (0x80..0xC0).contains(&byte)
}

fn decode_into(text: &mut String, bytes: &[u8], utf8: bool) {
    if utf8 {
        text.push_str(&String::from_utf8_lossy(bytes));
    } else {
        let (decoded, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
        text.push_str(&decoded);
    }
}

/// The lines of a matched block (one line, or several in a search across
/// lines): `(start, end)` of each line within `block`, terminator included.
pub(super) fn block_lines(block: &[u8]) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for newline in memchr::memchr_iter(b'\n', block) {
        lines.push((start, newline + 1));
        start = newline + 1;
    }
    if start < block.len() {
        lines.push((start, block.len()));
    }
    lines
}

/// `ranges` (within the block) clipped to the line `[start, end)` and moved
/// to the line's start; empty parts are dropped.
pub(super) fn clip_ranges(
    ranges: &[(usize, usize)],
    (start, end): (usize, usize),
) -> Vec<(usize, usize)> {
    ranges
        .iter()
        .filter_map(|&(from, to)| {
            let (from, to) = (from.max(start), to.min(end));
            (from < to).then(|| (from - start, to - start))
        })
        .collect()
}
