//! The replacement text of a regular expression as Notepad++ reads it:
//! Boost's `basic_regex_formatter` with `format_all` (Notepad++ calls
//! `match.format(…, boost::format_all)`, reference
//! docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md §1.1), evaluated here
//! step for step as Boost does: `$&` `$0` `$n` `${n}` `$+{name}` `` $` `` `$'`
//! `$+` `$$` and the Perl verbs (`$MATCH`, `${^MATCH}`, `$PREMATCH`,
//! `$POSTMATCH`, `$LAST_PAREN_MATCH`, `$LAST_SUBMATCH_RESULT`, `$^N`), the
//! escapes `\a \e \f \n \r \t \v \xDD \x{DDDD} \cX \0ooo \1`…`\9`, the case
//! switches `\l \u \L \U \E`, the conditionals `?n…:…`, `?{n}`, `?{name}` and
//! grouping with `( )`. Everything else is literal; an unmatched `)` ends
//! the text, as in Boost.
use super::content_replace_out::{Out, State};

/// Nesting of groups and conditionals the formatter follows; deeper text is
/// written literally (the recursion stays within any thread's stack).
const MAX_DEPTH: usize = 256;

/// The match a replacement text is evaluated for.
pub(super) struct View<'v> {
    pub(super) haystack: &'v [u8],
    /// Group spans; 0 = the whole match, `None` = did not take part.
    pub(super) groups: &'v [Option<(usize, usize)>],
    /// Where the search for this match began (start of `` $` ``).
    pub(super) prefix_start: usize,
    /// The number of a named group.
    pub(super) names: &'v dyn Fn(&str) -> Option<usize>,
}

/// Writes the replacement `pattern` for the match `view` into `out`.
pub(super) fn format(pattern: &[char], view: &View<'_>, out: &mut Out) {
    let mut formatter = Formatter {
        chars: pattern,
        pos: 0,
        conditional: false,
        depth: 0,
        view,
        out,
    };
    formatter.format_all();
}

struct Formatter<'f, 'v> {
    chars: &'f [char],
    pos: usize,
    /// Boost's `m_have_conditional`: a `:` ends the current part.
    conditional: bool,
    depth: usize,
    view: &'f View<'v>,
    out: &'f mut Out,
}

impl Formatter<'_, '_> {
    fn at(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }

    fn current(&self) -> Option<char> {
        self.at(self.pos)
    }

    fn put(&mut self, character: char) {
        self.out.put(character);
    }

    fn put_code(&mut self, code: i64) {
        let character = u32::try_from(code)
            .ok()
            .and_then(char::from_u32)
            .unwrap_or(char::REPLACEMENT_CHARACTER);
        self.put(character);
    }

    fn group(&self, index: i64) -> Option<(usize, usize)> {
        let index = usize::try_from(index).ok()?;
        self.view.groups.get(index).copied().flatten()
    }

    fn put_group(&mut self, index: i64) {
        let span = self.group(index);
        self.out.put_span(self.view.haystack, span);
    }

    fn whole(&self) -> (usize, usize) {
        self.group(0).unwrap_or((0, 0))
    }

    /// `$+`: the highest-numbered group (Boost: `m_results[size - 1]`, or 1).
    fn last_paren(&self) -> i64 {
        let size = self.view.groups.len();
        if size > 1 {
            size as i64 - 1
        } else {
            1
        }
    }

    /// `$^N`: the group that closed last. Without the matcher's own record:
    /// the participating group that ends last, of those the one that starts
    /// last, of nested groups with the same span the outer one.
    fn last_closed(&self) -> i64 {
        let mut best: Option<(usize, (usize, usize))> = None;
        for (index, span) in self.view.groups.iter().enumerate().skip(1) {
            let Some(span) = *span else { continue };
            let better = match best {
                None => true,
                Some((_, (start, end))) => span.1 > end || (span.1 == end && span.0 > start),
            };
            if better {
                best = Some((index, span));
            }
        }
        best.map_or(-1, |(index, _)| index as i64)
    }

    /// Boost's `toi`: up to `max` digits of `radix` from the position; -1
    /// without any (the position stays) or on overflow.
    fn toi(&mut self, max: usize, radix: u32) -> i64 {
        let end = self.pos.saturating_add(max).min(self.chars.len());
        if self.pos >= end || self.chars[self.pos].to_digit(radix).is_none() {
            return -1;
        }
        let limit = i64::from(i32::MAX) / i64::from(radix);
        let mut result = 0i64;
        while self.pos < end {
            let Some(digit) = self.chars[self.pos].to_digit(radix) else {
                break;
            };
            result = result * i64::from(radix) + i64::from(digit);
            self.pos += 1;
            if result > limit {
                return -1;
            }
        }
        result
    }

    fn format_all(&mut self) {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            while let Some(character) = self.current() {
                self.put(character);
                self.pos += 1;
            }
            self.depth -= 1;
            return;
        }
        while let Some(character) = self.current() {
            match character {
                '\\' => self.escape(),
                '(' => {
                    self.pos += 1;
                    let had = self.conditional;
                    self.conditional = false;
                    self.until_scope_end();
                    self.conditional = had;
                    if self.current().is_none() {
                        break;
                    }
                    self.pos += 1;
                }
                ')' => break,
                ':' if self.conditional => break,
                '?' => {
                    self.pos += 1;
                    self.condition();
                }
                '$' => self.perl(),
                other => {
                    self.put(other);
                    self.pos += 1;
                }
            }
        }
        self.depth -= 1;
    }

    fn until_scope_end(&mut self) {
        loop {
            self.format_all();
            match self.current() {
                None | Some(')') => return,
                Some(other) => {
                    self.put(other);
                    self.pos += 1;
                }
            }
        }
    }

    fn escape(&mut self) {
        self.pos += 1;
        let Some(kind) = self.current() else {
            self.put('\\');
            return;
        };
        let simple = match kind {
            'a' => Some('\x07'),
            'f' => Some('\x0C'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'v' => Some('\x0B'),
            'e' => Some('\x1B'),
            _ => None,
        };
        if let Some(character) = simple {
            self.put(character);
            self.pos += 1;
            return;
        }
        match kind {
            'x' => self.hex_escape(),
            'c' => {
                self.pos += 1;
                match self.current() {
                    None => self.put('c'),
                    Some(control) => {
                        self.pos += 1;
                        self.put_code(i64::from(u32::from(control) % 32));
                    }
                }
            }
            'l' | 'u' => {
                self.pos += 1;
                self.out.restore = self.out.state;
                self.out.state = if kind == 'l' {
                    State::NextLower
                } else {
                    State::NextUpper
                };
            }
            'L' | 'U' | 'E' => {
                self.pos += 1;
                self.out.state = match kind {
                    'L' => State::Lower,
                    'U' => State::Upper,
                    _ => State::Copy,
                };
            }
            _ => {
                let value = self.toi(1, 10);
                if value > 0 {
                    self.put_group(value);
                } else if value == 0 {
                    // `\0ooo`: an octal character code, `\0` itself = NUL.
                    self.pos -= 1;
                    let code = self.toi(4, 8);
                    self.put_code(code.max(0));
                } else {
                    self.put(kind);
                    self.pos += 1;
                }
            }
        }
    }

    /// `\xDD`, `\x{DDDD}`; Boost's handling of malformed forms.
    fn hex_escape(&mut self) {
        let x = self.pos;
        self.pos += 1;
        if self.current().is_none() {
            self.put('x');
            return;
        }
        if self.current() == Some('{') {
            self.pos += 1;
            let value = self.toi(usize::MAX, 16);
            if value < 0 {
                self.put('x');
                self.put('{');
                return;
            }
            if self.current() != Some('}') {
                self.pos = x + 1;
                self.put('x');
                return;
            }
            self.pos += 1;
            self.put_code(value);
            return;
        }
        let value = self.toi(2, 16);
        if value < 0 {
            self.pos = x + 1;
            self.put('x');
            return;
        }
        self.put_code(value);
    }

    fn perl(&mut self) {
        self.pos += 1;
        let Some(kind) = self.current() else {
            self.put('$');
            return;
        };
        let save = self.pos;
        match kind {
            '&' => {
                self.pos += 1;
                self.put_group(0);
            }
            '`' => {
                self.pos += 1;
                let span = (self.view.prefix_start.min(self.whole().0), self.whole().0);
                self.out.put_span(self.view.haystack, Some(span));
            }
            '\'' => {
                self.pos += 1;
                let span = (self.whole().1, self.view.haystack.len());
                self.out.put_span(self.view.haystack, Some(span));
            }
            '$' => {
                self.pos += 1;
                self.put('$');
            }
            '+' => {
                self.pos += 1;
                if self.current() == Some('{') {
                    let base = self.pos + 1;
                    let close = (base..self.chars.len()).find(|&index| self.chars[index] == '}');
                    if let Some(close) = close {
                        let name: String = self.chars[base..close].iter().collect();
                        let index = (self.view.names)(&name);
                        self.pos = close + 1;
                        if let Some(index) = index {
                            self.put_group(index as i64);
                        }
                        return;
                    }
                }
                let last = self.last_paren();
                self.put_group(last);
            }
            '{' => {
                self.pos += 1;
                self.perl_number(true, save);
            }
            _ => self.perl_number(false, save),
        }
    }

    fn perl_number(&mut self, brace: bool, save: usize) {
        let value = self.toi(usize::MAX, 10);
        if value < 0 || (brace && self.current() != Some('}')) {
            if !self.perl_verb(brace) {
                // Not a substitution: the `$` stays, the text goes on after it.
                self.pos = save;
                self.put('$');
            }
            return;
        }
        self.put_group(value);
        if brace {
            self.pos += 1;
        }
    }

    fn perl_verb(&mut self, brace: bool) -> bool {
        if self.current().is_none() {
            return false;
        }
        if brace && self.current() == Some('^') {
            self.pos += 1;
        }
        const VERBS: [&str; 6] = [
            "MATCH",
            "PREMATCH",
            "POSTMATCH",
            "LAST_PAREN_MATCH",
            "LAST_SUBMATCH_RESULT",
            "^N",
        ];
        for (number, verb) in VERBS.iter().enumerate() {
            let length = verb.chars().count();
            let fits = self.chars.len() >= self.pos + length
                && self.chars[self.pos..self.pos + length]
                    .iter()
                    .copied()
                    .eq(verb.chars());
            if !fits {
                continue;
            }
            self.pos += length;
            if brace {
                if self.current() == Some('}') {
                    self.pos += 1;
                } else {
                    return false;
                }
            }
            let (start, end) = self.whole();
            match number {
                0 => self.put_group(0),
                1 => {
                    let span = (self.view.prefix_start.min(start), start);
                    self.out.put_span(self.view.haystack, Some(span));
                }
                2 => {
                    let span = (end, self.view.haystack.len());
                    self.out.put_span(self.view.haystack, Some(span));
                }
                3 => {
                    let last = self.last_paren();
                    self.put_group(last);
                }
                _ => {
                    let last = self.last_closed();
                    self.put_group(last);
                }
            }
            return true;
        }
        false
    }

    /// `?n…:…`, `?{n}…`, `?{name}…` (after the `?`).
    fn condition(&mut self) {
        if self.current().is_none() {
            self.put('?');
            return;
        }
        let number = if self.current() == Some('{') {
            let base = self.pos;
            self.pos += 1;
            let mut value = self.toi(usize::MAX, 10);
            if value < 0 {
                let start = self.pos;
                while self.current().is_some_and(|c| c != '}') {
                    self.pos += 1;
                }
                let name: String = self.chars[start..self.pos].iter().collect();
                value = (self.view.names)(&name).map_or(-1, |index| index as i64);
            }
            if value < 0 || self.current() != Some('}') {
                self.pos = base;
                self.put('?');
                return;
            }
            self.pos += 1;
            value
        } else {
            self.toi(2, 10)
        };
        if number < 0 {
            self.put('?');
            return;
        }
        if self.group(number).is_some() {
            self.conditional = true;
            self.format_all();
            self.conditional = false;
            if self.current() == Some(':') {
                self.pos += 1;
                let saved = self.out.state;
                self.out.state = State::None;
                self.until_scope_end();
                self.out.state = saved;
            }
        } else {
            let saved = self.out.state;
            self.out.state = State::None;
            self.conditional = true;
            self.format_all();
            self.conditional = false;
            self.out.state = saved;
            if self.current() == Some(':') {
                self.pos += 1;
                self.until_scope_end();
            }
        }
    }
}
