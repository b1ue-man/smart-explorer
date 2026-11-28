//! Finding the spans to mask in bytes: literal and regex entries on the
//! bytes (`regex::bytes`), fuzzy entries on every valid UTF-8 section
//! (candidates of `fuzzy`, plus exact folded occurrences inside compounds
//! with one Aho-Corasick automaton). Overlapping hits of any entries merge
//! into their union, so no part of a value stays visible.
use aho_corasick::{AhoCorasick, MatchKind};

use super::admin::REGEX_SIZE_LIMIT;
use super::doc::EntryDoc;
use super::fold::{is_base_separator, tokens, Token, MAX_GAP_CHARS};
use super::fuzzy::FuzzyPattern;
use super::model::{EntryId, EntryKind, ListId, MaskError};

/// Window a stream holds for regex entries (their matches have no bound).
pub(crate) const REGEX_STREAM_WINDOW: usize = 64 * 1024;

pub(crate) struct CompiledEntry {
    pub list: ListId,
    pub entry: EntryId,
    pub label: String,
    pub kind: EntryKind,
    /// The value of a literal or fuzzy entry: what `⟦label-N⟧` stands for.
    pub value: Vec<u8>,
    regex: Option<regex::bytes::Regex>,
    whole_word: bool,
    fuzzy: Option<FuzzyPattern>,
}

/// A span to mask; `owner` indexes the entry that names its placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub owner: usize,
}

pub(crate) struct Matcher {
    entries: Vec<CompiledEntry>,
    exact: Option<(AhoCorasick, Vec<usize>)>,
    has_fuzzy: bool,
    hold: usize,
}

impl Matcher {
    /// A matcher without entries.
    pub(crate) fn empty() -> Self {
        Self {
            entries: Vec::new(),
            exact: None,
            has_fuzzy: false,
            hold: 0,
        }
    }

    pub(crate) fn compile<'a>(
        entries: impl IntoIterator<Item = (ListId, &'a EntryDoc)>,
    ) -> Result<Self, MaskError> {
        let mut compiled = Vec::new();
        let mut exact_patterns = Vec::new();
        let mut exact_owner = Vec::new();
        let mut hold = 0usize;
        for (list, entry) in entries {
            let mut item = CompiledEntry {
                list,
                entry: entry.id,
                label: entry.label.clone(),
                kind: entry.kind,
                value: entry.value.as_bytes().to_vec(),
                regex: None,
                whole_word: entry.whole_word,
                fuzzy: None,
            };
            match entry.kind {
                EntryKind::Literal => {
                    let forms = written_forms(&entry.value, true);
                    item.regex = Some(literal_regex(&forms, entry.case_sensitive, entry.id)?);
                    hold = hold.max(forms_hold(&forms));
                }
                EntryKind::Regex => {
                    let regex = regex::bytes::RegexBuilder::new(&entry.value)
                        .size_limit(REGEX_SIZE_LIMIT)
                        .build()
                        .map_err(|e| {
                            MaskError::Unreadable(format!("Eintrag {}: {e}", entry.id.0))
                        })?;
                    item.regex = Some(regex);
                    hold = hold.max(REGEX_STREAM_WINDOW);
                }
                EntryKind::Fuzzy => {
                    let Some(pattern) = FuzzyPattern::new(&entry.value, entry.tolerance) else {
                        continue;
                    };
                    exact_patterns.push(pattern.folded());
                    exact_owner.push(compiled.len());
                    hold = hold.max(pattern.max_len() * (8 + 4 * MAX_GAP_CHARS) + 64);
                    item.fuzzy = Some(pattern);
                    // The JSON-escaped spellings exactly (Groß/Klein ignored);
                    // the folding cannot see through `\\n` or `\\"`.
                    let forms = written_forms(&entry.value, false);
                    if !forms.is_empty() {
                        item.regex = Some(literal_regex(&forms, false, entry.id)?);
                        hold = hold.max(forms_hold(&forms));
                    }
                }
            }
            compiled.push(item);
        }
        let exact = if exact_patterns.is_empty() {
            None
        } else {
            let automaton = AhoCorasick::builder()
                .match_kind(MatchKind::Standard)
                .build(&exact_patterns)
                .map_err(|e| MaskError::Unreadable(format!("unscharfe Einträge: {e}")))?;
            Some((automaton, exact_owner))
        };
        Ok(Self {
            has_fuzzy: exact.is_some(),
            entries: compiled,
            exact,
            hold,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn entry(&self, index: usize) -> &CompiledEntry {
        &self.entries[index]
    }

    /// Bytes a stream must hold back so that every match that starts before
    /// the cut is complete.
    pub(crate) fn hold(&self) -> usize {
        self.hold
    }

    /// The merged spans of `bytes`, sorted and disjoint.
    pub(crate) fn find(&self, bytes: &[u8]) -> Vec<Span> {
        let mut hits: Vec<Span> = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let Some(regex) = &entry.regex else {
                continue;
            };
            for found in regex.find_iter(bytes) {
                if found.start() == found.end() {
                    continue;
                }
                if entry.whole_word && !whole_word_at(bytes, found.start(), found.end()) {
                    continue;
                }
                hits.push(Span {
                    start: found.start(),
                    end: found.end(),
                    owner: index,
                });
            }
        }
        if self.has_fuzzy {
            for (start, text) in utf8_sections(bytes) {
                self.find_fuzzy(text, start, &mut hits);
            }
        }
        merge(hits)
    }

    fn find_fuzzy(&self, text: &str, base: usize, hits: &mut Vec<Span>) {
        let tokens = tokens(text, base);
        if tokens.is_empty() {
            return;
        }
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(pattern) = &entry.fuzzy {
                for hit in pattern.find(text, base, &tokens) {
                    hits.push(Span {
                        start: hit.start,
                        end: hit.end,
                        owner: index,
                    });
                }
            }
        }
        if let Some((automaton, owners)) = &self.exact {
            exact_in_groups(text, base, &tokens, automaton, owners, hits);
        }
    }
}

/// The spellings of a value in outputs: the value itself (with `plain`)
/// and, when JSON escaping changes it, its escaped form once and twice
/// (`se … --json` writes `C:\\Users`, JSON inside JSON `C:\\\\Users`).
/// A found escaped form differs from the value and is therefore kept as a
/// variant: unmasking restores the escaped spelling unchanged.
pub(crate) fn written_forms(value: &str, plain: bool) -> Vec<String> {
    let mut forms = Vec::new();
    if plain {
        forms.push(value.to_string());
    }
    let once = json_escaped(value);
    if once != value {
        let twice = json_escaped(&once);
        forms.push(once);
        forms.push(twice);
    }
    // Longest first: the alternation of `regex` prefers earlier branches.
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    forms.dedup();
    forms
}

/// The body of a JSON string as serde_json writes it.
fn json_escaped(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_default();
    quoted
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}

fn literal_regex(
    forms: &[String],
    case_sensitive: bool,
    entry: EntryId,
) -> Result<regex::bytes::Regex, MaskError> {
    let pattern = forms
        .iter()
        .map(|form| regex::escape(form))
        .collect::<Vec<_>>()
        .join("|");
    regex::bytes::RegexBuilder::new(&format!("(?:{pattern})"))
        .case_insensitive(!case_sensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| MaskError::Unreadable(format!("Eintrag {}: {e}", entry.0)))
}

fn forms_hold(forms: &[String]) -> usize {
    forms.iter().map(String::len).max().unwrap_or(0) * 3 + 8
}

/// Exact folded occurrences inside runs of words joined by separators. A
/// match inside one word may start anywhere (compounds); a match over
/// several words must start at a word start, so it never borrows the end
/// of the word before (reference §2.2, „sweinbraten“ from „es Weinbraten“).
fn exact_in_groups(
    text: &str,
    base: usize,
    tokens: &[Token],
    automaton: &AhoCorasick,
    owners: &[usize],
    hits: &mut Vec<Span>,
) {
    let mut first = 0;
    while first < tokens.len() {
        let mut last = first;
        while last + 1 < tokens.len() && joined(text, base, &tokens[last], &tokens[last + 1]) {
            last += 1;
        }
        let group = &tokens[first..=last];
        let mut folded = String::new();
        // Per folded character: byte offset in `folded`, token index, origin.
        let mut chars: Vec<(usize, usize, (usize, usize))> = Vec::new();
        let mut token_first = Vec::with_capacity(group.len());
        for (token_index, token) in group.iter().enumerate() {
            token_first.push(chars.len());
            for (c, origin) in token.folded.iter().zip(&token.origin) {
                chars.push((folded.len(), token_index, *origin));
                folded.push(*c);
            }
        }
        for found in automaton.find_overlapping_iter(&folded) {
            let start_char = chars.partition_point(|c| c.0 < found.start());
            let end_char = chars.partition_point(|c| c.0 < found.end());
            if start_char >= end_char || end_char > chars.len() {
                continue;
            }
            let (_, first_token, start_origin) = chars[start_char];
            let (_, last_token, end_origin) = chars[end_char - 1];
            if first_token != last_token && token_first[first_token] != start_char {
                continue;
            }
            hits.push(Span {
                start: start_origin.0,
                end: end_origin.1,
                owner: owners[found.pattern().as_usize()],
            });
        }
        first = last + 1;
    }
}

fn joined(text: &str, base: usize, left: &Token, right: &Token) -> bool {
    let gap = &text[left.end - base..right.start - base];
    let count = gap.chars().count();
    count > 0 && count <= MAX_GAP_CHARS && gap.chars().all(is_base_separator)
}

/// Unions of overlapping hits; the longest hit of a union (then the first
/// entry) names the placeholder.
fn merge(mut hits: Vec<Span>) -> Vec<Span> {
    hits.sort_by_key(|hit| (hit.start, std::cmp::Reverse(hit.end), hit.owner));
    let mut merged: Vec<(Span, usize)> = Vec::new();
    for hit in hits {
        let len = hit.end - hit.start;
        match merged.last_mut() {
            Some((current, longest)) if hit.start < current.end => {
                if len > *longest || (len == *longest && hit.owner < current.owner) {
                    current.owner = hit.owner;
                    *longest = len;
                }
                current.end = current.end.max(hit.end);
            }
            _ => merged.push((hit, len)),
        }
    }
    merged.into_iter().map(|(span, _)| span).collect()
}

/// The maximal valid UTF-8 sections of `bytes` with their offsets.
pub(crate) fn utf8_sections(bytes: &[u8]) -> Vec<(usize, &str)> {
    let mut sections = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(text) => {
                sections.push((offset, text));
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    if let Ok(text) = std::str::from_utf8(&bytes[offset..offset + valid]) {
                        sections.push((offset, text));
                    }
                }
                offset += valid
                    + error
                        .error_len()
                        .unwrap_or(bytes.len() - offset - valid)
                        .max(1);
            }
        }
    }
    sections
}

/// The character that ends at `pos`, if `pos` follows a valid one.
pub(crate) fn char_before(bytes: &[u8], pos: usize) -> Option<char> {
    (1..=4.min(pos)).find_map(|len| {
        std::str::from_utf8(&bytes[pos - len..pos])
            .ok()
            .and_then(|text| text.chars().next())
    })
}

/// The character that starts at `pos`.
pub(crate) fn char_at(bytes: &[u8], pos: usize) -> Option<char> {
    let end = (pos + 4).min(bytes.len());
    (pos + 1..=end).find_map(|stop| {
        std::str::from_utf8(&bytes[pos..stop])
            .ok()
            .and_then(|text| text.chars().next())
    })
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn whole_word_at(bytes: &[u8], start: usize, end: usize) -> bool {
    !is_word(char_before(bytes, start)) && !is_word(char_at(bytes, end))
}
