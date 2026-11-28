//! Path globs of `se search` and the MCP `search` tool (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C "Coding-Lücken"): a
//! glob with `/` or `**` is matched against the whole path relative to the
//! searched folder, as Claude Code's Glob and globset with a literal
//! separator match it. `*` and `?` stay within one path component, a
//! component `**` stands for any number of components (none included),
//! `[abc]`, `[a-z]`, `[!a]`/`[^a]` are character classes, `{a,b}` are
//! alternatives (also nested), `\x` is the character `x`. Matching ignores
//! letter case, as the name globs do; a leading `./` or `/` is dropped. A glob
//! without `/` and `**` stays a name glob (`search::glob_match`).

/// Alternatives a glob may expand to (`{a,b}{c,d}` = 4).
const MAX_ALTERNATIVES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Char(char),
    /// `?`: one character.
    One,
    /// `*`: any characters of one component.
    Star,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    /// `**`: any number of components.
    AnyDepth,
    Part(Vec<Token>),
}

/// A parsed path glob.
#[derive(Clone, Debug)]
pub(super) struct PathGlob {
    alternatives: Vec<Vec<Segment>>,
}

/// Whether `query` is a path glob rather than a name glob.
pub(super) fn is_path_glob(query: &str) -> bool {
    query.contains('/') || query.contains("**")
}

/// `{a,b}` alternatives of `pattern`, outermost first; an unmatched brace is
/// a literal character.
fn expand(pattern: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut depth = 0usize;
    let mut open = None;
    let mut commas = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 1,
            '{' => {
                if depth == 0 {
                    open = Some(index);
                    commas.clear();
                }
                depth += 1;
            }
            ',' if depth == 1 => commas.push(index),
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let start = open.unwrap_or(0);
                    let prefix: String = chars[..start].iter().collect();
                    let suffix: String = chars[index + 1..].iter().collect();
                    let mut bounds = vec![start];
                    bounds.extend(&commas);
                    bounds.push(index);
                    let mut out = Vec::new();
                    for pair in bounds.windows(2) {
                        let middle: String = chars[pair[0] + 1..pair[1]].iter().collect();
                        for item in expand(&format!("{prefix}{middle}{suffix}"))? {
                            out.push(item);
                            if out.len() > MAX_ALTERNATIVES {
                                return Err(format!(
                                    "the glob has more than {MAX_ALTERNATIVES} alternatives"
                                ));
                            }
                        }
                    }
                    return Ok(out);
                }
            }
            _ => {}
        }
        index += 1;
    }
    Ok(vec![pattern.to_string()])
}

fn tokens(part: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = part.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        index += 1;
        match character {
            '?' => out.push(Token::One),
            '*' => {
                if out.last() != Some(&Token::Star) {
                    out.push(Token::Star);
                }
            }
            '\\' => match chars.get(index) {
                Some(next) => {
                    out.push(Token::Char(*next));
                    index += 1;
                }
                None => out.push(Token::Char('\\')),
            },
            '[' => {
                let mut cursor = index;
                let negated = matches!(chars.get(cursor), Some('!' | '^'));
                if negated {
                    cursor += 1;
                }
                let mut ranges = Vec::new();
                let mut first = true;
                let mut closed = false;
                while cursor < chars.len() {
                    let low = chars[cursor];
                    if low == ']' && !first {
                        closed = true;
                        cursor += 1;
                        break;
                    }
                    first = false;
                    if chars.get(cursor + 1) == Some(&'-')
                        && chars.get(cursor + 2).is_some_and(|high| *high != ']')
                    {
                        let high = chars[cursor + 2];
                        if high < low {
                            return Err(format!("invalid range `{low}-{high}` in the glob"));
                        }
                        ranges.push((low, high));
                        cursor += 3;
                    } else {
                        ranges.push((low, low));
                        cursor += 1;
                    }
                }
                if !closed {
                    return Err("unclosed `[` in the glob".to_string());
                }
                out.push(Token::Class { negated, ranges });
                index = cursor;
            }
            other => out.push(Token::Char(other)),
        }
    }
    Ok(out)
}

fn segments(pattern: &str) -> Result<Vec<Segment>, String> {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let pattern = pattern.trim_start_matches('/');
    let mut out: Vec<Segment> = Vec::new();
    for part in pattern.split('/').filter(|part| !part.is_empty()) {
        if part == "**" {
            if out.last() != Some(&Segment::AnyDepth) {
                out.push(Segment::AnyDepth);
            }
        } else {
            out.push(Segment::Part(tokens(&part.to_lowercase())?));
        }
    }
    if out.is_empty() {
        return Err("the glob names no path".to_string());
    }
    Ok(out)
}

fn class_has(negated: bool, ranges: &[(char, char)], character: char) -> bool {
    let inside = ranges.iter().any(|(low, high)| {
        (*low..=*high).contains(&character)
            || low
                .to_lowercase()
                .zip(high.to_lowercase())
                .any(|(low, high)| (low..=high).contains(&character))
    });
    inside != negated
}

/// Whether `tokens` match the whole component `text` (lower case).
fn component(tokens: &[Token], text: &[char]) -> bool {
    // matched[j]: the tokens so far match text[..j].
    let mut matched = vec![false; text.len() + 1];
    matched[0] = true;
    for token in tokens {
        let mut next = vec![false; text.len() + 1];
        for j in 0..=text.len() {
            match token {
                Token::Star => next[j] = matched[j] || (j > 0 && next[j - 1]),
                _ if j == 0 => {}
                Token::One => next[j] = matched[j - 1],
                Token::Char(character) => next[j] = matched[j - 1] && text[j - 1] == *character,
                Token::Class { negated, ranges } => {
                    next[j] = matched[j - 1] && class_has(*negated, ranges, text[j - 1])
                }
            }
        }
        matched = next;
    }
    matched[text.len()]
}

fn path_matches(pattern: &[Segment], parts: &[Vec<char>]) -> bool {
    match pattern.split_first() {
        None => parts.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=parts.len()).any(|skip| path_matches(rest, &parts[skip..]))
        }
        Some((Segment::Part(tokens), rest)) => parts
            .split_first()
            .is_some_and(|(first, others)| component(tokens, first) && path_matches(rest, others)),
    }
}

impl PathGlob {
    pub(super) fn parse(pattern: &str) -> Result<Self, String> {
        let alternatives = expand(pattern)?
            .iter()
            .map(|alternative| segments(alternative))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { alternatives })
    }

    /// Whether the path `rel` (relative to the searched folder,
    /// `/`-separated) matches.
    pub(super) fn matches(&self, rel: &str) -> bool {
        let parts: Vec<Vec<char>> = rel
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| part.to_lowercase().chars().collect())
            .collect();
        self.alternatives
            .iter()
            .any(|pattern| path_matches(pattern, &parts))
    }

    /// The name glob every match's last component matches (only `*`, `?` and
    /// plain characters), for a search that filters names next to the data;
    /// `None` when the last components differ or need more.
    pub(super) fn name_hint(&self) -> Option<String> {
        let mut hint: Option<String> = None;
        for pattern in &self.alternatives {
            let Some(Segment::Part(tokens)) = pattern.last() else {
                return None;
            };
            let mut text = String::new();
            for token in tokens {
                match token {
                    Token::Char('*' | '?') => return None,
                    Token::Char(character) => text.push(*character),
                    Token::One => text.push('?'),
                    Token::Star => text.push('*'),
                    Token::Class { .. } => return None,
                }
            }
            match &hint {
                Some(known) if *known != text => return None,
                _ => hint = Some(text),
            }
        }
        hint
    }
}

#[cfg(test)]
mod tests {
    use super::{is_path_glob, PathGlob};

    #[test]
    fn mcp_coding_task_path_globs_match_the_relative_path() {
        let glob = PathGlob::parse("src/**/*.rs").unwrap();
        assert!(glob.matches("src/main.rs"));
        assert!(glob.matches("src/a/b/lib.RS"));
        assert!(!glob.matches("tests/main.rs"));
        assert!(!glob.matches("src/main.rs.bak"));
        let any = PathGlob::parse("**/*.{ts,tsx}").unwrap();
        assert!(any.matches("a.ts") && any.matches("web/ui/b.tsx"));
        assert!(!any.matches("web/c.js"));
        assert_eq!(any.name_hint(), None, "two different last components");
        let star = PathGlob::parse("docs/*.md").unwrap();
        assert!(star.matches("docs/README.md"));
        assert!(!star.matches("docs/sub/x.md"), "`*` stays in one component");
        assert_eq!(star.name_hint().as_deref(), Some("*.md"));
        let class = PathGlob::parse("./[a-c]?/[!x]*").unwrap();
        assert!(class.matches("b1/yes") && !class.matches("d1/yes") && !class.matches("b1/xno"));
        let below = PathGlob::parse("target/**").unwrap();
        assert!(below.matches("target") && below.matches("target/debug/se"));
        let escaped = PathGlob::parse("a/\\*.txt").unwrap();
        assert!(escaped.matches("a/*.txt") && !escaped.matches("a/b.txt"));
        assert!(PathGlob::parse("a/[b").is_err());
        assert!(PathGlob::parse("/").is_err());
        assert!(PathGlob::parse(&"{a,b}".repeat(11)).is_err());
        assert!(is_path_glob("src/*.rs") && is_path_glob("**.rs") && !is_path_glob("*.rs"));
    }
}
