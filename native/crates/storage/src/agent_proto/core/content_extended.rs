//! The pattern texts of a content search as the matchers read them:
//! Notepad++'s "Extended" escapes, the Boost spellings Notepad++ users write in
//! regular expressions, the escaping of a literal, and whether a pattern can
//! match a line end (then the search runs across lines).

/// `\R` of Boost: any line break, `\r\n` as one.
const LINE_BREAK: &str = r"(?:\r\n|[\n\x0B\x0C\r\x{85}\x{2028}\x{2029}])";

/// The engine a translated regular expression is meant for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// Rust `regex` (through `grep-regex`), in CRLF mode (`(?R)`).
    Regex,
    /// `fancy-regex`, the backtracking fallback: `$` and `.` spelled out so
    /// they keep the CRLF meaning.
    Fancy { dot_matches_newline: bool },
}

/// Notepad++ "Extended" mode: `\n`, `\r`, `\t`, `\0`, `\\`, `\xHH`, `\uHHHH`,
/// `\oOOO` (octal), `\dDDD` (decimal), `\bBBBBBBBB` (binary) become characters.
/// An unknown escape, or one without its digits, stays as written.
pub(super) fn unescape_extended(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current != '\\' || index + 1 >= chars.len() {
            out.push(current);
            index += 1;
            continue;
        }
        let kind = chars[index + 1];
        let simple = match kind {
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            '0' => Some('\0'),
            '\\' => Some('\\'),
            _ => None,
        };
        if let Some(character) = simple {
            out.push(character);
            index += 2;
            continue;
        }
        let digits = match kind {
            'x' => Some((16, 2)),
            'u' => Some((16, 4)),
            'o' => Some((8, 3)),
            'd' => Some((10, 3)),
            'b' => Some((2, 8)),
            _ => None,
        };
        let value = digits.and_then(|(radix, count)| {
            let digits: String = chars.get(index + 2..index + 2 + count)?.iter().collect();
            let value = u32::from_str_radix(&digits, radix).ok()?;
            let limit = if radix == 16 && count == 4 {
                0xFFFF
            } else {
                0xFF
            };
            char::from_u32(value)
                .filter(|_| value <= limit || radix == 8)
                .map(|character| (character, count))
        });
        match value {
            Some((character, count)) => {
                out.push(character);
                index += 2 + count;
            }
            None => {
                out.push('\\');
                out.push(kind);
                index += 2;
            }
        }
    }
    out
}

/// `text` as a regular expression that matches exactly it.
pub(super) fn escape_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for character in text.chars() {
        if r"\.+*?()|[]{}^$#&-~".contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

/// Notepad++'s (Boost) spellings in Rust syntax: `\R`, `\<`, `\>`, `` \` ``,
/// `\'`; for the fancy engine also `$` (before `\r\n`, `\n`, `\r` or the end)
/// and `.` (never `\r`/`\n` unless ". matches newline"). Character classes
/// are copied unchanged.
pub(super) fn translate_boost(pattern: &str, target: Target) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut class = 0usize;
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current == '\\' {
            let Some(&next) = chars.get(index + 1) else {
                out.push('\\');
                break;
            };
            index += 2;
            if class > 0 {
                out.push('\\');
                out.push(next);
                continue;
            }
            match (next, target) {
                ('R', _) => out.push_str(LINE_BREAK),
                ('<', Target::Regex) => out.push_str(r"\b{start}"),
                ('>', Target::Regex) => out.push_str(r"\b{end}"),
                ('<', Target::Fancy { .. }) => out.push_str(r"\b(?=\w)"),
                ('>', Target::Fancy { .. }) => out.push_str(r"\b(?<=\w)"),
                ('`', _) => out.push_str(r"\A"),
                ('\'', _) => out.push_str(r"\z"),
                _ => {
                    out.push('\\');
                    out.push(next);
                }
            }
            continue;
        }
        index += 1;
        match current {
            '[' if class > 0 && chars.get(index) == Some(&':') => {
                // A POSIX class `[:name:]` inside a class: copied whole.
                out.push('[');
                while index < chars.len() {
                    out.push(chars[index]);
                    index += 1;
                    if chars[index - 1] == ']' && chars.get(index - 2) == Some(&':') {
                        break;
                    }
                }
            }
            '[' => {
                class += 1;
                out.push('[');
                if chars.get(index) == Some(&'^') {
                    out.push('^');
                    index += 1;
                }
                if class == 1 && chars.get(index) == Some(&']') {
                    out.push(']');
                    index += 1;
                }
            }
            ']' if class > 0 => {
                class -= 1;
                out.push(']');
            }
            '$' if class == 0 && matches!(target, Target::Fancy { .. }) => {
                out.push_str(r"(?=\r\n|\n|\r|\z)");
            }
            '.' if class == 0
                && target
                    == (Target::Fancy {
                        dot_matches_newline: false,
                    }) =>
            {
                out.push_str(r"[^\r\n]");
            }
            other => out.push(other),
        }
    }
    out
}

/// Whether a (translated) regular expression may match a line end, so the
/// search must run across lines as in Notepad++. Conservative: a construct
/// that might match `\n` counts (escapes such as `\n`, `\s`, `\W`, `\D`, `\P`,
/// `\x0A`, a negated class, a class with code escapes, POSIX space/control
/// classes, an inline `s` flag, a literal line feed).
pub(super) fn can_match_newline(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut class = 0usize;
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        index += 1;
        match current {
            '\n' => return true,
            '\\' => {
                let Some(&next) = chars.get(index) else {
                    return false;
                };
                index += 1;
                match next {
                    'n' | 'r' | 's' | 'W' | 'D' | 'P' | 'R' => return true,
                    'x' | 'u' | '0' if class > 0 => return true,
                    'x' if hex_escape_is_line_feed(&chars[index..]) => return true,
                    _ => {}
                }
            }
            '[' => {
                if class > 0 && chars.get(index) == Some(&':') {
                    let rest: String = chars[index..].iter().take(9).collect();
                    if rest.starts_with(":space:")
                        || rest.starts_with(":cntrl:")
                        || rest.starts_with(":^")
                    {
                        return true;
                    }
                    continue;
                }
                class += 1;
                if chars.get(index) == Some(&'^') {
                    return true;
                }
            }
            ']' if class > 0 => class -= 1,
            '(' if class == 0 && chars.get(index) == Some(&'?') => {
                let flags: String = chars[index + 1..]
                    .iter()
                    .take_while(|character| character.is_ascii_alphabetic())
                    .collect();
                if flags.contains('s') {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// The digits after `\x`: `0A`, `{A}`, `{000a}` and the like name a line feed.
fn hex_escape_is_line_feed(rest: &[char]) -> bool {
    let digits: String = if rest.first() == Some(&'{') {
        rest[1..].iter().take_while(|c| **c != '}').collect()
    } else {
        rest.iter().take(2).collect()
    };
    u32::from_str_radix(&digits, 16).is_ok_and(|value| value == 0x0A)
}
