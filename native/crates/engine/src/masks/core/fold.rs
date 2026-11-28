//! Folding and word tokens for fuzzy entries (plan S2, reference
//! docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md §2.2): lower case,
//! accents removed through NFKD, ä→ae, ö→oe, ü→ue, ß→ss, æ→ae, œ→oe. Every
//! folded character keeps the byte range of the original character it came
//! from, so a match maps back onto the original text exactly.
use icu_normalizer::DecomposingNormalizerBorrowed;

/// Most separator characters allowed between two words of one match.
pub(crate) const MAX_GAP_CHARS: usize = 16;

/// Combining marks (dropped by the folding, `¨` after a/o/u becomes `e`).
pub(crate) fn is_combining(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}

/// Characters of a word token.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining(c)
}

/// Separators a fuzzy match may contain between its words (plan S2):
/// white space with line endings, `-`, soft hyphen U+00AD, U+2010–U+2013.
pub(crate) fn is_base_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '\u{00AD}' | '\u{2010}'..='\u{2013}')
}

/// Appends the folded form of `c` (possibly nothing) to `out`.
pub(crate) fn fold_char(c: char, out: &mut Vec<char>) {
    if is_combining(c) {
        push_combining(c, out);
        return;
    }
    for lower in c.to_lowercase() {
        push_lower(lower, out);
    }
}

fn push_combining(c: char, out: &mut Vec<char>) {
    if c == '\u{0308}' && matches!(out.last(), Some('a' | 'o' | 'u')) {
        out.push('e');
    }
}

fn push_lower(c: char, out: &mut Vec<char>) {
    match c {
        'ß' => out.extend(['s', 's']),
        'ä' | 'æ' => out.extend(['a', 'e']),
        'ö' | 'œ' => out.extend(['o', 'e']),
        'ü' => out.extend(['u', 'e']),
        'ø' => out.push('o'),
        'ł' => out.push('l'),
        'đ' => out.push('d'),
        'ı' => out.push('i'),
        c if c.is_ascii() => {
            if c.is_ascii_alphanumeric() {
                out.push(c);
            }
        }
        c => {
            let normalizer = DecomposingNormalizerBorrowed::new_nfkd();
            for part in normalizer.normalize_iter(std::iter::once(c)) {
                if is_combining(part) {
                    push_combining(part, out);
                    continue;
                }
                for lower in part.to_lowercase() {
                    match lower {
                        'ß' => out.extend(['s', 's']),
                        lower if lower.is_alphanumeric() => out.push(lower),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// The folded word characters of `text` (everything else dropped): the
/// pattern of a fuzzy entry.
pub(crate) fn fold_pattern(text: &str) -> Vec<char> {
    let mut out = Vec::new();
    for c in text.chars().filter(|c| is_word_char(*c)) {
        fold_char(c, &mut out);
    }
    out
}

/// Non-word, non-separator characters of an entry value (`.` in
/// „Dr. Müller“): they may stand between the words of a match of it.
pub(crate) fn extra_gap_chars(text: &str) -> Vec<char> {
    let mut chars: Vec<char> = text
        .chars()
        .filter(|c| !is_word_char(*c) && !is_base_separator(*c))
        .collect();
    chars.sort_unstable();
    chars.dedup();
    chars
}

/// One word of the text: its byte range, its folded characters and, per
/// folded character, the byte range of the original character.
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub start: usize,
    pub end: usize,
    pub folded: Vec<char>,
    pub origin: Vec<(usize, usize)>,
}

/// The word tokens of `text`; byte offsets are relative to `text` plus
/// `base`.
pub(crate) fn tokens(text: &str, base: usize) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current: Option<Token> = None;
    for (offset, c) in text.char_indices() {
        let start = base + offset;
        let end = start + c.len_utf8();
        if is_word_char(c) {
            let token = current.get_or_insert_with(|| Token {
                start,
                end,
                folded: Vec::new(),
                origin: Vec::new(),
            });
            let before = token.folded.len();
            fold_char(c, &mut token.folded);
            for _ in before..token.folded.len() {
                token.origin.push((start, end));
            }
            token.end = end;
        } else if let Some(token) = current.take() {
            if !token.folded.is_empty() {
                tokens.push(token);
            }
        }
    }
    if let Some(token) = current.filter(|token| !token.folded.is_empty()) {
        tokens.push(token);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(text: &str) -> String {
        fold_pattern(text).into_iter().collect()
    }

    #[test]
    fn masks_task_folding_unifies_case_umlauts_sharp_s_and_accents() {
        let cases = [
            ("Schwein Braten", "schweinbraten"),
            ("Schweins-\nbraten", "schweinsbraten"),
            ("Straße", "strasse"),
            ("STRASSE", "strasse"),
            ("Müller", "mueller"),
            ("Mueller", "mueller"),
            ("Mu\u{0308}ller", "mueller"),
            ("Ärger", "aerger"),
            ("café", "cafe"),
            ("ﬁnal", "final"),
            ("Œuvre", "oeuvre"),
        ];
        for (input, expected) in cases {
            assert_eq!(fold(input), expected, "{input}");
        }
    }

    #[test]
    fn masks_task_tokens_keep_original_byte_ranges() {
        let text = "Ein Müller-Haus";
        let tokens = tokens(text, 10);
        let words: Vec<&str> = tokens
            .iter()
            .map(|t| &text[t.start - 10..t.end - 10])
            .collect();
        assert_eq!(words, ["Ein", "Müller", "Haus"]);
        let muller = &tokens[1];
        assert_eq!(muller.folded.iter().collect::<String>(), "mueller");
        // `ü` folds to two characters that both point at its two bytes.
        assert_eq!(muller.origin[1], muller.origin[2]);
        assert_eq!(muller.origin[1].1 - muller.origin[1].0, 2);
    }
}
