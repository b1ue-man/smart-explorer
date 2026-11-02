//! The ignore patterns of a sync job in the form a walk next to the data
//! evaluates them (`walk_filter.rs`): the regular expression globset 0.4.18
//! forms from each glob (`Glob::regex`, built with the options of
//! `syncjobs::compile_patterns` on the device that runs the sync), read back
//! here into a small automaton over bytes. The client takes the text from
//! globset itself, so the glob syntax, its escape rule (which differs between
//! Windows and the other systems) and its `**` rules are globset's own.
//!
//! The meaning is that of globset's matcher (`regex-automata` 0.4 with
//! `regex-syntax` 0.8): Unicode off (`(?-u)`), `.` is any byte (globset sets
//! `dot_matches_new_line`), no UTF-8 requirement, letter case folded for
//! ASCII letters only under `(?i)` (a class is folded before it is negated,
//! `bytes_fold_and_negate`), the whole path anchored by `^…$`.
//!
//! Accepted are exactly the constructs globset writes: `(?-u)`, an optional
//! `(?i)`, `^`, then literals (a raw ASCII character that is no regex meta
//! character, a backslash before ASCII punctuation other than `<` and `>`,
//! `\xNN`), `.`, bracket
//! classes of such literals and ranges (`[^…]` negated), `(?:…|…)` groups and
//! the quantifiers `*` and `?`, then `$`. Any other text is refused: the walk
//! next to the data then does not apply that pattern and the client still
//! does (`walk_filter::host_patterns`).
use std::cell::RefCell;
use std::io;

/// States of the automaton of one filter (all its patterns).
pub const MAX_STATES: usize = 1 << 16;
/// Group nesting a pattern may have.
const MAX_DEPTH: usize = 64;

/// A set of byte values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bytes([u64; 4]);

impl Bytes {
    const NONE: Bytes = Bytes([0; 4]);
    const ALL: Bytes = Bytes([u64::MAX; 4]);

    fn insert(&mut self, byte: u8) {
        self.0[usize::from(byte >> 6)] |= 1u64 << (byte & 63);
    }

    fn insert_range(&mut self, low: u8, high: u8) {
        for byte in low..=high {
            self.insert(byte);
        }
    }

    fn contains(&self, byte: u8) -> bool {
        self.0[usize::from(byte >> 6)] & (1u64 << (byte & 63)) != 0
    }

    /// Adds the other case of every ASCII letter in the set
    /// (`ClassBytesRange::case_fold_simple`).
    fn fold_ascii(&mut self) {
        for byte in b'a'..=b'z' {
            if self.contains(byte) {
                self.insert(byte - 32);
            }
        }
        for byte in b'A'..=b'Z' {
            if self.contains(byte) {
                self.insert(byte + 32);
            }
        }
    }

    fn negate(&mut self) {
        for word in &mut self.0 {
            *word = !*word;
        }
    }
}

/// A parsed pattern.
#[derive(Debug)]
enum Node {
    Byte(Bytes),
    Star(Box<Node>),
    Optional(Box<Node>),
    Sequence(Vec<Node>),
    Either(Vec<Node>),
}

fn refused(why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("ignore pattern not evaluated next to the data: {why}"),
    )
}

/// Characters regex-syntax gives a meaning outside a class.
fn is_meta(byte: u8) -> bool {
    b"\\.+*?()|[]{}^$".contains(&byte)
}

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
    fold: bool,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.text.get(self.at + ahead).copied()
    }

    fn either(&mut self) -> io::Result<Node> {
        let mut branches = vec![self.sequence()?];
        while self.peek() == Some(b'|') {
            self.at += 1;
            branches.push(self.sequence()?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Node::Either(branches)
        })
    }

    fn sequence(&mut self) -> io::Result<Node> {
        let mut items = Vec::new();
        while !matches!(self.peek(), None | Some(b'|') | Some(b')')) {
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(Node::Sequence(items))
    }

    fn quantified(&mut self, atom: Node) -> io::Result<Node> {
        let node = match self.peek() {
            Some(b'*') => Node::Star(Box::new(atom)),
            Some(b'?') => Node::Optional(Box::new(atom)),
            _ => return Ok(atom),
        };
        self.at += 1;
        if matches!(self.peek(), Some(b'*' | b'?' | b'+' | b'{')) {
            return Err(refused("repeated quantifier"));
        }
        Ok(node)
    }

    fn atom(&mut self) -> io::Result<Node> {
        let byte = self.peek().ok_or_else(|| refused("pattern ends early"))?;
        match byte {
            b'(' => {
                if self.text.get(self.at..self.at + 3) != Some(&b"(?:"[..]) {
                    return Err(refused("group other than (?:"));
                }
                self.at += 3;
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(refused("groups nested too deep"));
                }
                let inner = self.either()?;
                if self.peek() != Some(b')') {
                    return Err(refused("unclosed group"));
                }
                self.at += 1;
                self.depth -= 1;
                Ok(inner)
            }
            b'[' => self.class(),
            b'.' => {
                self.at += 1;
                Ok(Node::Byte(Bytes::ALL))
            }
            b'\\' => {
                let literal = self.escaped()?;
                Ok(Node::Byte(self.literal(literal)))
            }
            byte if byte < 0x80 && !is_meta(byte) => {
                self.at += 1;
                Ok(Node::Byte(self.literal(byte)))
            }
            _ => Err(refused("unexpected character")),
        }
    }

    /// A literal byte, both cases of an ASCII letter under `(?i)`.
    fn literal(&self, byte: u8) -> Bytes {
        let mut set = Bytes::NONE;
        set.insert(byte);
        if self.fold && byte.is_ascii_alphabetic() {
            set.insert(byte ^ 0x20);
        }
        set
    }

    /// The byte after a backslash: `\xNN` or escaped ASCII punctuation.
    fn escaped(&mut self) -> io::Result<u8> {
        self.at += 1;
        let next = self.peek().ok_or_else(|| refused("dangling escape"))?;
        if next == b'x' {
            let digits = self
                .text
                .get(self.at + 1..self.at + 3)
                .and_then(|digits| std::str::from_utf8(digits).ok())
                .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .ok_or_else(|| refused("escape other than \\xNN"))?;
            let byte = u8::from_str_radix(digits, 16).map_err(|_| refused("bad \\xNN"))?;
            self.at += 3;
            return Ok(byte);
        }
        // `\<` and `\>` are word boundaries in regex-syntax 0.8.
        if next.is_ascii_punctuation() && next != b'<' && next != b'>' {
            self.at += 1;
            return Ok(next);
        }
        Err(refused("escape class"))
    }

    /// One literal of a bracket class.
    fn class_literal(&mut self) -> io::Result<u8> {
        match self.peek() {
            Some(b'\\') => self.escaped(),
            Some(byte) if byte < 0x80 && !b"[]\\^-&~".contains(&byte) => {
                self.at += 1;
                Ok(byte)
            }
            _ => Err(refused("unexpected character in a class")),
        }
    }

    fn class(&mut self) -> io::Result<Node> {
        self.at += 1;
        let negated = self.peek() == Some(b'^');
        if negated {
            self.at += 1;
        }
        let mut set = Bytes::NONE;
        let mut first = true;
        loop {
            match self.peek() {
                None => return Err(refused("unclosed class")),
                Some(b']') if !first => {
                    self.at += 1;
                    break;
                }
                _ => {}
            }
            let low = self.class_literal()?;
            if self.peek() == Some(b'-') {
                if self.peek_at(1) == Some(b']') {
                    return Err(refused("range without its end"));
                }
                self.at += 1;
                let high = self.class_literal()?;
                if low > high {
                    return Err(refused("reversed range"));
                }
                set.insert_range(low, high);
            } else {
                set.insert(low);
            }
            first = false;
        }
        if self.fold {
            set.fold_ascii();
        }
        if negated {
            set.negate();
        }
        Ok(Node::Byte(set))
    }
}

/// The pattern of one globset regular expression.
fn parse(regex: &str) -> io::Result<Node> {
    let text = regex
        .as_bytes()
        .strip_prefix(b"(?-u)")
        .ok_or_else(|| refused("no (?-u) prefix"))?;
    let (fold, text) = match text.strip_prefix(b"(?i)") {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let body = text
        .strip_prefix(b"^")
        .and_then(|rest| rest.strip_suffix(b"$"))
        .ok_or_else(|| refused("not anchored"))?;
    let mut parser = Parser {
        text: body,
        at: 0,
        fold,
        depth: 0,
    };
    let node = parser.either()?;
    if parser.at != body.len() {
        return Err(refused("unexpected text"));
    }
    Ok(node)
}

#[derive(Clone, Copy, Debug)]
enum State {
    Byte(Bytes, usize),
    Split(usize, usize),
    Match,
}

/// The automaton of several patterns: a path matches when any one does.
#[derive(Debug)]
pub struct PatternSet {
    states: Vec<State>,
    start: Option<usize>,
}

fn push(states: &mut Vec<State>, state: State) -> io::Result<usize> {
    if states.len() >= MAX_STATES {
        return Err(refused("patterns too large"));
    }
    states.push(state);
    Ok(states.len() - 1)
}

/// The states of `node` followed by `next`; returns its first state.
fn compile(node: &Node, next: usize, states: &mut Vec<State>) -> io::Result<usize> {
    Ok(match node {
        Node::Byte(set) => push(states, State::Byte(*set, next))?,
        Node::Sequence(items) => {
            let mut first = next;
            for item in items.iter().rev() {
                first = compile(item, first, states)?;
            }
            first
        }
        Node::Either(branches) => {
            let mut starts = Vec::with_capacity(branches.len());
            for branch in branches {
                starts.push(compile(branch, next, states)?);
            }
            let mut first = starts.pop().unwrap_or(next);
            while let Some(other) = starts.pop() {
                first = push(states, State::Split(other, first))?;
            }
            first
        }
        Node::Optional(inner) => {
            let body = compile(inner, next, states)?;
            push(states, State::Split(body, next))?
        }
        Node::Star(inner) => {
            let looped = push(states, State::Split(next, next))?;
            let body = compile(inner, looped, states)?;
            states[looped] = State::Split(body, next);
            looped
        }
    })
}

/// The states one pattern takes (to keep a filter within `MAX_STATES`);
/// refused when the walk next to the data cannot evaluate it.
pub fn pattern_states(regex: &str) -> io::Result<usize> {
    let mut states = vec![State::Match];
    compile(&parse(regex)?, 0, &mut states)?;
    Ok(states.len())
}

/// The work lists of one thread, reused between paths.
#[derive(Default)]
struct Scratch {
    marks: Vec<u32>,
    generation: u32,
    current: Vec<usize>,
    next: Vec<usize>,
    stack: Vec<usize>,
}

impl Scratch {
    fn begin(&mut self, states: usize) {
        if self.marks.len() < states {
            self.marks.resize(states, 0);
        }
        self.step();
    }

    fn step(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.marks.iter_mut().for_each(|mark| *mark = 0);
            self.generation = 1;
        }
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

impl PatternSet {
    /// All of `regexes`; refused as a whole when one is.
    pub fn new(regexes: &[String]) -> io::Result<PatternSet> {
        let mut states = vec![State::Match];
        let mut starts = Vec::with_capacity(regexes.len());
        for regex in regexes {
            starts.push(compile(&parse(regex)?, 0, &mut states)?);
        }
        let mut start = starts.pop();
        while let Some(other) = starts.pop() {
            if let Some(first) = start {
                start = Some(push(&mut states, State::Split(other, first))?);
            }
        }
        Ok(PatternSet { states, start })
    }

    pub fn is_empty(&self) -> bool {
        self.start.is_none()
    }

    /// Whether any pattern matches the whole of `path`.
    pub fn is_match(&self, path: &[u8]) -> bool {
        let Some(start) = self.start else {
            return false;
        };
        SCRATCH.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            let scratch = &mut *scratch;
            scratch.begin(self.states.len());
            scratch.current.clear();
            self.close(start, scratch, true);
            for &byte in path {
                scratch.step();
                scratch.next.clear();
                let current = std::mem::take(&mut scratch.current);
                for &state in &current {
                    if let State::Byte(set, next) = self.states[state] {
                        if set.contains(byte) {
                            self.close(next, scratch, false);
                        }
                    }
                }
                scratch.current = current;
                std::mem::swap(&mut scratch.current, &mut scratch.next);
                if scratch.current.is_empty() {
                    return false;
                }
            }
            scratch
                .current
                .iter()
                .any(|&state| matches!(self.states[state], State::Match))
        })
    }

    /// Adds `state` and every state it reaches without a byte to the current
    /// (`into_current`) or the next list.
    fn close(&self, state: usize, scratch: &mut Scratch, into_current: bool) {
        scratch.stack.clear();
        scratch.stack.push(state);
        while let Some(state) = scratch.stack.pop() {
            if scratch.marks[state] == scratch.generation {
                continue;
            }
            scratch.marks[state] = scratch.generation;
            match self.states[state] {
                State::Split(first, second) => {
                    scratch.stack.push(second);
                    scratch.stack.push(first);
                }
                State::Byte(..) | State::Match => {
                    if into_current {
                        scratch.current.push(state);
                    } else {
                        scratch.next.push(state);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "walk_pattern_tests.rs"]
mod tests;
