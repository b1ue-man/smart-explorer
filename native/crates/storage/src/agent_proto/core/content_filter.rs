//! Which folders a content search enters and which files it reads: the
//! Notepad++ "Filters" field exactly (plan docs/plaene/2026-11-27-dateien-suche-ansicht
//! section B1) plus the folder-path regular expressions of the user
//! (`ContentQuery::folder_include`, `folder_exclude`).
//!
//! Filters: patterns separated by white space, no quoting; `*` and `?` match
//! the name case-insensitively (DOS semantics: `*.*` = every name, `name.*` also
//! a name without extension); `!pattern` excludes files and wins over every
//! inclusion; `!\pattern` excludes folders of the first level below the root,
//! `!+\pattern` folders of every level. Empty, or only exclusions, = `*.*` plus
//! the exclusions.
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};

use super::content_query::ContentQuery;

/// One wildcard pattern, lower-cased once.
#[derive(Clone, Debug)]
struct Wildcard {
    chars: Vec<char>,
    /// The pattern ends in `.*`: a name without a dot matches the part before it
    /// (DOS: `*.*` matches `README`, `make.*` matches `make`).
    dos_stem: Option<Vec<char>>,
}

impl Wildcard {
    fn new(pattern: &str) -> Self {
        let chars: Vec<char> = pattern.to_lowercase().chars().collect();
        let dos_stem = (chars.len() >= 2 && chars.ends_with(&['.', '*']))
            .then(|| chars[..chars.len() - 2].to_vec());
        Self { chars, dos_stem }
    }

    fn matches(&self, name: &str) -> bool {
        let name: Vec<char> = name.to_lowercase().chars().collect();
        if wildcard_match(&self.chars, &name) {
            return true;
        }
        match &self.dos_stem {
            Some(stem) if !name.contains(&'.') => wildcard_match(stem, &name),
            _ => false,
        }
    }
}

/// `*` = any run of characters, `?` = exactly one character.
fn wildcard_match(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < name.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[t]) {
            p += 1;
            t += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// The parsed filters and folder expressions of one content search.
#[derive(Clone, Debug)]
pub struct PathFilter {
    includes: Vec<Wildcard>,
    file_excludes: Vec<Wildcard>,
    top_excludes: Vec<Wildcard>,
    any_excludes: Vec<Wildcard>,
    folder_include: Option<RegexMatcher>,
    folder_exclude: Option<RegexMatcher>,
    subfolders: bool,
    hidden: bool,
}

impl PathFilter {
    /// The filter of `query`; the error names an invalid folder expression.
    pub fn new(query: &ContentQuery) -> Result<Self, String> {
        let mut filter = Self {
            includes: Vec::new(),
            file_excludes: Vec::new(),
            top_excludes: Vec::new(),
            any_excludes: Vec::new(),
            folder_include: folder_regex(query.folder_include.as_deref(), "einschließen")?,
            folder_exclude: folder_regex(query.folder_exclude.as_deref(), "ausschließen")?,
            subfolders: query.subfolders,
            hidden: query.hidden,
        };
        for token in query.filters.split_whitespace() {
            let (list, pattern) = if let Some(pattern) = token.strip_prefix("!+\\") {
                (&mut filter.any_excludes, pattern)
            } else if let Some(pattern) = token.strip_prefix("!\\") {
                (&mut filter.top_excludes, pattern)
            } else if let Some(pattern) = token.strip_prefix('!') {
                (&mut filter.file_excludes, pattern)
            } else {
                (&mut filter.includes, token)
            };
            if !pattern.is_empty() {
                list.push(Wildcard::new(pattern));
            }
        }
        Ok(filter)
    }

    /// Whether the search enters the folder `rel_folder` (relative to the
    /// root, `/`-separated) named `name` at `level` (1 = directly below the
    /// root); `hidden` = the folder is hidden.
    pub fn admit_folder(&self, rel_folder: &str, level: usize, name: &str, hidden: bool) -> bool {
        if !self.subfolders || (hidden && !self.hidden) {
            return false;
        }
        if level == 1
            && self
                .top_excludes
                .iter()
                .any(|pattern| pattern.matches(name))
        {
            return false;
        }
        if self
            .any_excludes
            .iter()
            .any(|pattern| pattern.matches(name))
        {
            return false;
        }
        !self
            .folder_exclude
            .as_ref()
            .is_some_and(|regex| regex_matches(regex, rel_folder))
    }

    /// Whether the files directly in `rel_folder` ("" = the root) are searched
    /// (`folder_include`; the walk goes on below a folder that does not match).
    pub fn search_in_folder(&self, rel_folder: &str) -> bool {
        self.folder_include
            .as_ref()
            .is_none_or(|regex| regex_matches(regex, rel_folder))
    }

    /// Whether a file named `name` is searched: an exclusion always wins.
    pub fn admit_file(&self, name: &str) -> bool {
        if self
            .file_excludes
            .iter()
            .any(|pattern| pattern.matches(name))
        {
            return false;
        }
        self.includes.is_empty() || self.includes.iter().any(|pattern| pattern.matches(name))
    }
}

/// A folder expression; case-insensitive like the Notepad++ filters (an
/// expression can switch that off with `(?-i)`).
fn folder_regex(pattern: Option<&str>, which: &str) -> Result<Option<RegexMatcher>, String> {
    let Some(pattern) = pattern.filter(|pattern| !pattern.is_empty()) else {
        return Ok(None);
    };
    RegexMatcherBuilder::new()
        .case_insensitive(true)
        .build(pattern)
        .map(Some)
        .map_err(|error| format!("Ungültiger regulärer Ausdruck für „Ordner {which}“: {error}"))
}

fn regex_matches(regex: &RegexMatcher, text: &str) -> bool {
    regex.is_match(text.as_bytes()).unwrap_or(false)
}

#[cfg(test)]
#[path = "content_filter_tests.rs"]
mod tests;
