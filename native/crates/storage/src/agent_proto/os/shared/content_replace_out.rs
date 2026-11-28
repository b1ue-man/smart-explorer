//! The output of a replacement while it is built: the new bytes in the
//! haystack's domain (UTF-8, or Windows-1252 bytes of an ANSI file) and
//! Boost's output state, which the case switches `\l \u \L \U \E` and the
//! suppressed branch of a conditional set.
use super::content_replace_text::{ansi_byte, ansi_char, Domain};

/// Boost's output states (`output_copy`, `output_none`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Copy,
    None,
    NextLower,
    NextUpper,
    Lower,
    Upper,
}

/// The new text of a file while it is built, in the haystack's domain.
pub(super) struct Out {
    pub(super) bytes: Vec<u8>,
    domain: Domain,
    pub(super) state: State,
    pub(super) restore: State,
    /// A character Windows-1252 lacks was written into an ANSI haystack.
    pub(super) unmappable: bool,
}

impl Out {
    pub(super) fn new(domain: Domain) -> Self {
        Self {
            bytes: Vec::new(),
            domain,
            state: State::Copy,
            restore: State::Copy,
            unmappable: false,
        }
    }

    /// Text between matches, unchanged.
    pub(super) fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// Every replacement starts with plain copying, as every Boost format call.
    pub(super) fn begin_match(&mut self) {
        self.state = State::Copy;
        self.restore = State::Copy;
    }

    /// A literal replacement (Normal, Extended).
    pub(super) fn literal(&mut self, text: &str) {
        match self.domain {
            Domain::Utf8 => self.bytes.extend_from_slice(text.as_bytes()),
            Domain::Ansi => text.chars().for_each(|c| self.push(c)),
        }
    }

    pub(super) fn put(&mut self, character: char) {
        let character = match self.state {
            State::None => return,
            State::Copy => character,
            State::NextLower => {
                self.state = self.restore;
                lower(character)
            }
            State::NextUpper => {
                self.state = self.restore;
                upper(character)
            }
            State::Lower => lower(character),
            State::Upper => upper(character),
        };
        self.push(character);
    }

    fn push(&mut self, character: char) {
        match self.domain {
            Domain::Utf8 => {
                let mut buffer = [0u8; 4];
                self.bytes
                    .extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            }
            Domain::Ansi => match ansi_byte(character) {
                Some(byte) => self.bytes.push(byte),
                None => self.unmappable = true,
            },
        }
    }

    /// Text of the haystack (a group, the prefix, the suffix).
    pub(super) fn put_span(&mut self, haystack: &[u8], span: Option<(usize, usize)>) {
        let Some((start, end)) = span else {
            return;
        };
        let bytes = &haystack[start.min(haystack.len())..end.min(haystack.len())];
        match self.domain {
            Domain::Utf8 => {
                for chunk in bytes.utf8_chunks() {
                    chunk.valid().chars().for_each(|c| self.put(c));
                    if self.state != State::None {
                        // Invalid bytes stay as they are.
                        self.bytes.extend_from_slice(chunk.invalid());
                    }
                }
            }
            Domain::Ansi => bytes.iter().for_each(|byte| self.put(ansi_char(*byte))),
        }
    }
}

/// One-to-one case mapping (Boost's `toupper`/`tolower` per character): a
/// character whose mapping has several characters stays.
fn upper(character: char) -> char {
    single(character.to_uppercase()).unwrap_or(character)
}

fn lower(character: char) -> char {
    single(character.to_lowercase()).unwrap_or(character)
}

fn single(mut mapped: impl Iterator<Item = char>) -> Option<char> {
    let first = mapped.next()?;
    mapped.next().is_none().then_some(first)
}
