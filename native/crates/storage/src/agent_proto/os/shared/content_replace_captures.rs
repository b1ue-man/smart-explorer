//! The matches of a replacement with their groups: the matcher of the content
//! search itself where `grep-regex` runs the pattern (its captures), and a
//! `fancy-regex` built exactly as `content_fancy.rs` builds it where the
//! backtracking engine runs it (that one offers no captures). Matches follow
//! `grep_matcher`'s iteration, as the search's: an empty match directly after
//! a match is not taken, and the next search starts one character further.
//! In a UTF-8 haystack no match splits a character.
use std::io;

use grep_matcher::{Captures, Matcher};

use super::content_extended::{translate_boost, Target};
use super::content_match::{ContentMatcher, Engine};
use super::content_query::ContentQuery;

/// Steps one match may take, as the search's backtracking matcher allows.
const BACKTRACK_LIMIT: usize = 1_000_000;

/// One match: group spans (0 = the whole match; `None` = did not take part)
/// and where the search that found it began (the start of Boost's `` $` ``).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Found {
    pub(super) groups: Vec<Option<(usize, usize)>>,
    pub(super) prefix_start: usize,
}

impl Found {
    pub(super) fn span(&self) -> (usize, usize) {
        self.groups.first().copied().flatten().unwrap_or((0, 0))
    }
}

/// Where the groups of a match come from.
#[derive(Debug)]
pub(super) enum Captor {
    /// The search's own `grep-regex` matcher.
    Grep,
    /// The backtracking engine, built as the search builds it.
    Fancy(fancy_regex::Regex),
}

impl Captor {
    pub(super) fn new(matcher: &ContentMatcher, query: &ContentQuery) -> Result<Self, String> {
        if !matcher.is_backtracking() {
            return Ok(Self::Grep);
        }
        // Only regular expressions reach the backtracking engine.
        let dot = query.dot_matches_newline;
        let pattern = translate_boost(
            &query.pattern,
            Target::Fancy {
                dot_matches_newline: dot,
            },
        );
        let mut flags = String::from("(?m");
        if !query.match_case {
            flags.push('i');
        }
        if dot {
            flags.push('s');
        }
        flags.push(')');
        fancy_regex::RegexBuilder::new(&format!("{flags}{pattern}"))
            .backtrack_limit(BACKTRACK_LIMIT)
            .build()
            .map(Self::Fancy)
            .map_err(|error| format!("Ungültiger regulärer Ausdruck: {error}"))
    }

    /// The number of the group called `name`.
    pub(super) fn group_index(&self, matcher: &ContentMatcher, name: &str) -> Option<usize> {
        match (self, matcher.engine()) {
            (Self::Fancy(regex), _) => regex.capture_names().position(|known| known == Some(name)),
            (Self::Grep, Engine::Regex(regex)) => regex.capture_index(name),
            (Self::Grep, Engine::Fancy(_)) => None,
        }
    }

    /// Every match in `haystack`, ascending and disjoint.
    pub(super) fn find_all(
        &self,
        matcher: &ContentMatcher,
        haystack: &[u8],
    ) -> io::Result<Vec<Found>> {
        let utf8 = std::str::from_utf8(haystack).is_ok();
        match (self, matcher.engine()) {
            (Self::Grep, Engine::Regex(regex)) => grep_all(regex, haystack, utf8),
            (Self::Fancy(regex), _) => fancy_all(regex, haystack),
            (Self::Grep, Engine::Fancy(_)) => Ok(Vec::new()),
        }
    }
}

/// The position after `at` where the next search starts: one character on.
fn step(haystack: &[u8], at: usize, utf8: bool) -> usize {
    if at >= haystack.len() {
        return haystack.len() + 1;
    }
    let mut next = at + 1;
    if utf8 {
        while next < haystack.len() && is_continuation(haystack[next]) {
            next += 1;
        }
    }
    next
}

fn is_continuation(byte: u8) -> bool {
    (0x80..0xC0).contains(&byte)
}

fn on_boundary(haystack: &[u8], at: usize, utf8: bool) -> bool {
    !utf8 || at >= haystack.len() || !is_continuation(haystack[at])
}

/// The iteration of `grep_matcher::Matcher::find_iter` with captures.
fn grep_all<M: Matcher>(regex: &M, haystack: &[u8], utf8: bool) -> io::Result<Vec<Found>> {
    let other = |error: M::Error| io::Error::other(error.to_string());
    let mut caps = regex.new_captures().map_err(other)?;
    let mut found = Vec::new();
    let mut at = 0usize;
    let mut last_end = None;
    while at <= haystack.len() {
        if !regex.captures_at(haystack, at, &mut caps).map_err(other)? {
            break;
        }
        let Some(whole) = caps.get(0) else {
            break;
        };
        let (start, end) = (whole.start(), whole.end());
        if !on_boundary(haystack, start, utf8) || !on_boundary(haystack, end, utf8) {
            at = step(haystack, start.max(at), utf8);
            continue;
        }
        if start == end && last_end == Some(end) {
            at = step(haystack, end, utf8);
            continue;
        }
        let groups = (0..caps.len())
            .map(|index| caps.get(index).map(|span| (span.start(), span.end())))
            .collect();
        found.push(Found {
            groups,
            prefix_start: at,
        });
        last_end = Some(end);
        at = if start == end {
            step(haystack, end, utf8)
        } else {
            end
        };
    }
    Ok(found)
}

/// The backtracking engine matches text: a haystack with invalid UTF-8 is
/// searched in its valid runs, as `content_fancy.rs` does.
fn fancy_all(regex: &fancy_regex::Regex, haystack: &[u8]) -> io::Result<Vec<Found>> {
    let mut found = Vec::new();
    if let Ok(text) = std::str::from_utf8(haystack) {
        fancy_run(regex, text, 0, &mut found)?;
        return Ok(found);
    }
    let mut base = 0usize;
    for chunk in haystack.utf8_chunks() {
        let valid = chunk.valid();
        fancy_run(regex, valid, base, &mut found)?;
        base += valid.len() + chunk.invalid().len();
    }
    Ok(found)
}

fn fancy_run(
    regex: &fancy_regex::Regex,
    text: &str,
    base: usize,
    found: &mut Vec<Found>,
) -> io::Result<()> {
    let bytes = text.as_bytes();
    let mut at = 0usize;
    let mut last_end = found.last().map(|last| last.span().1);
    while at <= text.len() {
        let caps = regex
            .captures_from_pos(text, at)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let Some(caps) = caps else {
            break;
        };
        let Some(whole) = caps.get(0) else {
            break;
        };
        let (start, end) = (whole.start(), whole.end());
        if start == end && last_end == Some(base + end) {
            at = step(bytes, end, true);
            continue;
        }
        let groups = (0..caps.len())
            .map(|index| {
                caps.get(index)
                    .map(|span| (base + span.start(), base + span.end()))
            })
            .collect();
        found.push(Found {
            groups,
            prefix_start: base + at,
        });
        last_end = Some(base + end);
        at = if start == end {
            step(bytes, end, true)
        } else {
            end
        };
    }
    Ok(())
}
