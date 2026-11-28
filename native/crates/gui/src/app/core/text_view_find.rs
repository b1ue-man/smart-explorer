//! Searching inside one file of the text view (Strg+F, F3/Umschalt+F3) and
//! „Gehe zu Zeile“ (Strg+G). The search reads the document in windows of
//! lines through `LineProvider`, never the whole file at once, and wraps
//! around at the ends like an editor. Free of `egui`.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::textview::LineProvider;

/// Lines read per step of a search.
const FIND_CHUNK: usize = 4096;

/// A match: zero-based line and byte range in that line's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct FindHit {
    pub(in crate::app) line: u64,
    pub(in crate::app) start: usize,
    pub(in crate::app) end: usize,
}

/// Where a search continues from: forward it looks at bytes from `byte` of
/// `line` on, backward at matches that end before `byte`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct FindFrom {
    pub(in crate::app) line: u64,
    pub(in crate::app) byte: usize,
}

/// What a finished search reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct FindOutcome {
    pub(in crate::app) hit: Option<FindHit>,
    /// The search went past an end of the file and started over.
    pub(in crate::app) wrapped: bool,
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Suche abgebrochen")
}

fn first_match(matcher: &regex::Regex, text: &str, from: usize) -> Option<(usize, usize)> {
    if from > text.len() || !text.is_char_boundary(from) {
        return None;
    }
    // Empty matches (`^`, `x*`) mark nothing: step past them.
    let mut position = from;
    while position <= text.len() {
        let found = matcher.find_at(text, position)?;
        if found.end() > found.start() {
            return Some((found.start(), found.end()));
        }
        let step = text[found.start()..].chars().next()?.len_utf8();
        position = found.start() + step;
    }
    None
}

fn last_match(matcher: &regex::Regex, text: &str, before: usize) -> Option<(usize, usize)> {
    matcher
        .find_iter(text)
        .take_while(|found| found.start() < before)
        .filter(|found| found.end() <= before && found.end() > found.start())
        .last()
        .map(|found| (found.start(), found.end()))
}

/// The next match after `from` (forward) or before it (backward). `known`
/// is the number of lines known so far (the index may still grow); a
/// forward search also reads past it until the file ends.
pub(in crate::app) fn find_next(
    lines: &(impl LineProvider + ?Sized),
    matcher: &regex::Regex,
    from: FindFrom,
    forward: bool,
    known: u64,
    cancel: &AtomicBool,
) -> io::Result<FindOutcome> {
    if forward {
        if let Some(hit) = scan_forward(lines, matcher, from.line, Some(from.byte), None, cancel)? {
            return Ok(FindOutcome {
                hit: Some(hit),
                wrapped: false,
            });
        }
        let hit = scan_forward(lines, matcher, 0, None, Some(from.line + 1), cancel)?;
        return Ok(FindOutcome { hit, wrapped: true });
    }
    if let Some(hit) = scan_backward(lines, matcher, from.line, Some(from.byte), 0, cancel)? {
        return Ok(FindOutcome {
            hit: Some(hit),
            wrapped: false,
        });
    }
    let last = known.max(from.line + 1).saturating_sub(1);
    let hit = scan_backward(lines, matcher, last, None, from.line, cancel)?;
    Ok(FindOutcome { hit, wrapped: true })
}

/// Lines `start..` (up to `end`, exclusive, or the end of the file); in the
/// first line only from byte `first_byte`.
fn scan_forward(
    lines: &(impl LineProvider + ?Sized),
    matcher: &regex::Regex,
    start: u64,
    first_byte: Option<usize>,
    end: Option<u64>,
    cancel: &AtomicBool,
) -> io::Result<Option<FindHit>> {
    let mut line = start;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let want = end.map_or(FIND_CHUNK as u64, |end| {
            end.saturating_sub(line).min(FIND_CHUNK as u64)
        });
        if want == 0 {
            return Ok(None);
        }
        let batch = lines.provide_lines(line, want as usize)?;
        if batch.is_empty() {
            return Ok(None);
        }
        for (offset, text) in batch.iter().enumerate() {
            let number = line + offset as u64;
            let from = if number == start {
                first_byte.unwrap_or(0)
            } else {
                0
            };
            if let Some((match_start, match_end)) = first_match(matcher, text, from) {
                return Ok(Some(FindHit {
                    line: number,
                    start: match_start,
                    end: match_end,
                }));
            }
        }
        line += batch.len() as u64;
    }
}

/// Lines `start` down to `stop` (inclusive); in the first line only matches
/// ending before byte `first_byte`.
fn scan_backward(
    lines: &(impl LineProvider + ?Sized),
    matcher: &regex::Regex,
    start: u64,
    first_byte: Option<usize>,
    stop: u64,
    cancel: &AtomicBool,
) -> io::Result<Option<FindHit>> {
    let mut upper = start + 1; // exclusive
    while upper > stop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let lower = upper.saturating_sub(FIND_CHUNK as u64).max(stop);
        let batch = lines.provide_lines(lower, (upper - lower) as usize)?;
        for (offset, text) in batch.iter().enumerate().rev() {
            let number = lower + offset as u64;
            let before = if number == start {
                first_byte.unwrap_or(text.len())
            } else {
                text.len()
            };
            if let Some((match_start, match_end)) = last_match(matcher, text, before) {
                return Ok(Some(FindHit {
                    line: number,
                    start: match_start,
                    end: match_end,
                }));
            }
        }
        upper = lower;
    }
    Ok(None)
}

/// „Gehe zu Zeile“: the zero-based line of the 1-based number the user typed.
/// `total` is the line count once the index is complete; before that any
/// number is accepted (the view clamps it to the lines known).
pub(in crate::app) fn parse_goto(input: &str, total: Option<u64>) -> Result<u64, String> {
    let digits: String = input
        .trim()
        .chars()
        .filter(|ch| *ch != '.' && *ch != ' ')
        .collect();
    if digits.is_empty() {
        return Err("Bitte eine Zeilennummer eingeben.".to_string());
    }
    let number: u64 = digits
        .parse()
        .map_err(|_| format!("„{}“ ist keine Zeilennummer.", input.trim()))?;
    if number == 0 {
        return Err("Die Zeilen beginnen bei 1.".to_string());
    }
    if let Some(total) = total {
        if number > total {
            return Err(format!(
                "Die Datei hat nur {} Zeilen.",
                super::content_search_results::group_thousands(total)
            ));
        }
    }
    Ok(number - 1)
}
