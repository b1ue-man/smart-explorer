//! The matcher of a content search: ripgrep's `grep-regex` (literal
//! prefilters, SIMD, line-oriented fast path) for every pattern Rust's `regex`
//! runs, and the backtracking `fancy-regex` for the Boost/Perl features it
//! lacks. Notepad++ semantics: Normal and Extended are literals (Extended after
//! its escapes), "Match whole word only" for them, `^`/`$` at line ends
//! (`\r\n` included) and a search across lines where the pattern can match a
//! line end or ". matches newline" is on.
use grep_matcher::{LineTerminator, Matcher};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};

use super::content_extended::{
    can_match_newline, escape_literal, translate_boost, unescape_extended, Target,
};
use super::content_fancy::FancyMatcher;
use super::content_query::{ContentQuery, TextMode};

/// The engine behind a `ContentMatcher`.
#[derive(Debug)]
pub(super) enum Engine {
    Regex(RegexMatcher),
    Fancy(FancyMatcher),
}

/// The compiled pattern of one content search; shared by every searching
/// thread.
#[derive(Debug)]
pub struct ContentMatcher {
    engine: Engine,
    multi_line: bool,
    /// The pattern has characters beyond ASCII: a file in an ANSI code page is
    /// also searched transcoded from Windows-1252 (Notepad++ finds them there).
    non_ascii: bool,
}

impl ContentMatcher {
    /// The matcher of `query`; the error (German, for the user) names an empty
    /// or invalid pattern.
    pub fn new(query: &ContentQuery) -> Result<Self, String> {
        let text = match query.mode {
            TextMode::Extended => unescape_extended(&query.pattern),
            TextMode::Normal | TextMode::Regex => query.pattern.clone(),
        };
        if text.is_empty() {
            return Err("Der Suchtext ist leer.".to_string());
        }
        let non_ascii = !text.is_ascii();
        let case_insensitive = !query.match_case;
        if query.mode != TextMode::Regex {
            let multi_line = text.contains('\n');
            let pattern = escape_literal(&text);
            let options = Options {
                case_insensitive,
                word: query.whole_word,
                multi_line,
                dot_matches_newline: false,
            };
            let matcher = build_regex(&pattern, options)
                .map_err(|error| format!("Ungültiger Suchtext: {error}"))?;
            return Ok(Self {
                engine: Engine::Regex(matcher),
                multi_line,
                non_ascii,
            });
        }
        let dot_matches_newline = query.dot_matches_newline;
        let translated = translate_boost(&text, Target::Regex);
        let multi_line = dot_matches_newline || can_match_newline(&translated);
        let mut options = Options {
            case_insensitive,
            word: false,
            multi_line,
            dot_matches_newline,
        };
        let mut built = build_regex(&translated, options);
        if built.is_err() && !multi_line {
            // A line end the scan did not see (grep-regex refuses it in a
            // line-oriented search): search across lines.
            options.multi_line = true;
            if let Ok(matcher) = build_regex(&translated, options) {
                built = Ok(matcher);
            }
        }
        let regex_error = match built {
            Ok(matcher) => {
                return Ok(Self {
                    engine: Engine::Regex(matcher),
                    multi_line: options.multi_line,
                    non_ascii,
                })
            }
            Err(error) => error,
        };
        let fancy_pattern = translate_boost(
            &text,
            Target::Fancy {
                dot_matches_newline,
            },
        );
        let fancy_multi = dot_matches_newline || can_match_newline(&fancy_pattern);
        match FancyMatcher::new(&fancy_pattern, case_insensitive, dot_matches_newline) {
            Ok(matcher) => Ok(Self {
                engine: Engine::Fancy(matcher),
                multi_line: fancy_multi,
                non_ascii,
            }),
            Err(fancy_error) => {
                // A construct only the backtracking engine knows: its error is
                // the one that names the mistake.
                let error = if regex_error.contains("not supported") {
                    fancy_error
                } else {
                    regex_error
                };
                Err(format!("Ungültiger regulärer Ausdruck: {error}"))
            }
        }
    }

    /// The search runs across lines (the whole file at once).
    pub fn multi_line(&self) -> bool {
        self.multi_line
    }

    /// Whether the backtracking engine runs the pattern.
    pub fn is_backtracking(&self) -> bool {
        matches!(self.engine, Engine::Fancy(_))
    }

    pub(super) fn non_ascii(&self) -> bool {
        self.non_ascii
    }

    pub(super) fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The line terminator the searcher splits lines by: the matcher's own
    /// where it reports one (grep-searcher refuses a different one), else
    /// `\r\n` for the line-by-line fallback of the backtracking engine (the
    /// searcher then hands it lines without `\r\n`).
    pub(super) fn line_terminator(&self) -> LineTerminator {
        match &self.engine {
            Engine::Regex(matcher) => matcher
                .line_terminator()
                .unwrap_or_else(|| LineTerminator::byte(b'\n')),
            Engine::Fancy(_) if !self.multi_line => LineTerminator::crlf(),
            Engine::Fancy(_) => LineTerminator::byte(b'\n'),
        }
    }

    /// Byte ranges of every match in `haystack`, ascending and disjoint.
    pub(super) fn ranges(&self, haystack: &[u8]) -> std::io::Result<Vec<(usize, usize)>> {
        let mut ranges = Vec::new();
        let mut push = |found: grep_matcher::Match| {
            ranges.push((found.start(), found.end()));
            true
        };
        match &self.engine {
            Engine::Regex(matcher) => matcher
                .find_iter(haystack, &mut push)
                .map_err(|error| std::io::Error::other(error.to_string()))?,
            Engine::Fancy(matcher) => matcher
                .find_iter(haystack, &mut push)
                .map_err(std::io::Error::other)?,
        }
        Ok(ranges)
    }
}

#[derive(Clone, Copy)]
struct Options {
    case_insensitive: bool,
    word: bool,
    multi_line: bool,
    dot_matches_newline: bool,
}

/// `pattern` in CRLF mode (`(?R)`: `$` before `\r\n`, `.` never `\r`), `^`/`$`
/// at line ends; line-oriented (`\n` cannot match) unless `multi_line`.
fn build_regex(pattern: &str, options: Options) -> Result<RegexMatcher, String> {
    let mut builder = RegexMatcherBuilder::new();
    builder
        .case_insensitive(options.case_insensitive)
        .multi_line(true)
        .dot_matches_new_line(options.dot_matches_newline)
        .word(options.word)
        .octal(false);
    if !options.multi_line {
        builder.line_terminator(Some(b'\n'));
    }
    builder
        .build(&format!("(?R){pattern}"))
        .map_err(|error| error.to_string())
}
