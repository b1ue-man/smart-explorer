//! The backtracking matcher of a content search: `fancy-regex` behind the
//! `grep_matcher::Matcher` interface, for the regular expressions Rust's
//! `regex` cannot run (look-around, back references, atomic groups, `\K`).
//! It matches text, so a haystack with invalid UTF-8 is searched in its valid
//! runs (a pattern never matches the invalid bytes anyway).
use grep_matcher::{Match, Matcher, NoCaptures};

/// Steps one match may take before the file is reported as unreadable instead
/// of the search hanging on a catastrophic pattern (fancy-regex's default).
const BACKTRACK_LIMIT: usize = 1_000_000;

#[derive(Debug)]
pub(super) struct FancyMatcher {
    regex: fancy_regex::Regex,
}

impl FancyMatcher {
    /// `pattern` (already translated for this engine) with `^`/`$` at line
    /// ends; `case_insensitive` and `dot_matches_newline` as inline flags.
    pub(super) fn new(
        pattern: &str,
        case_insensitive: bool,
        dot_matches_newline: bool,
    ) -> Result<Self, String> {
        let mut flags = String::from("(?m");
        if case_insensitive {
            flags.push('i');
        }
        if dot_matches_newline {
            flags.push('s');
        }
        flags.push(')');
        let regex = fancy_regex::RegexBuilder::new(&format!("{flags}{pattern}"))
            .backtrack_limit(BACKTRACK_LIMIT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { regex })
    }

    fn find_in(&self, text: &str, at: usize) -> Result<Option<(usize, usize)>, String> {
        let mut at = at.min(text.len());
        while !text.is_char_boundary(at) {
            at += 1;
        }
        self.regex
            .find_from_pos(text, at)
            .map(|found| found.map(|found| (found.start(), found.end())))
            .map_err(|error| error.to_string())
    }
}

impl Matcher for FancyMatcher {
    type Captures = NoCaptures;
    type Error = String;

    fn find_at(&self, haystack: &[u8], at: usize) -> Result<Option<Match>, String> {
        if let Ok(text) = std::str::from_utf8(haystack) {
            return Ok(self
                .find_in(text, at)?
                .map(|(start, end)| Match::new(start, end)));
        }
        let mut base = 0usize;
        for chunk in haystack.utf8_chunks() {
            let valid = chunk.valid();
            let end = base + valid.len();
            if at <= end {
                if let Some((start, stop)) = self.find_in(valid, at.saturating_sub(base))? {
                    return Ok(Some(Match::new(base + start, base + stop)));
                }
            }
            base = end + chunk.invalid().len();
        }
        Ok(None)
    }

    fn new_captures(&self) -> Result<NoCaptures, String> {
        Ok(NoCaptures::new())
    }
}
