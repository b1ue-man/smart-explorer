//! The encoding of a file a replacement changes, kept as Notepad++ keeps it
//! (reference docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md §1.3): a BOM
//! stays, UTF-8 stays UTF-8, UTF-16 (with BOM) is replaced in its UTF-8 text
//! and written back as UTF-16, an ANSI file (no BOM, not valid UTF-8) is
//! Windows-1252 and refuses a replacement with characters that code page
//! lacks. Line ends are never touched: only the matched bytes change. A file
//! with NUL bytes is no text file (the search's binary detection).
use std::borrow::Cow;
use std::sync::OnceLock;

use super::content_replace::ReplaceSkip;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// What the bytes of a haystack are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Domain {
    /// UTF-8 (where the file is UTF-8, possibly with invalid bytes after a
    /// BOM, or decoded UTF-16/ANSI text).
    Utf8,
    /// The raw bytes of an ANSI file, one Windows-1252 character each.
    Ansi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Utf8 { bom: bool },
    Utf16 { big_endian: bool },
    Ansi,
}

/// A file's content with its encoding.
pub(super) struct Text<'c> {
    content: &'c [u8],
    encoding: Encoding,
}

/// What the matcher runs on.
pub(super) struct Haystack<'c> {
    pub(super) bytes: Cow<'c, [u8]>,
    pub(super) domain: Domain,
    /// Decoded text that is encoded again when written (UTF-16, ANSI).
    transcoded: bool,
}

/// The encoding of `content`, or why it stays unchanged.
pub(super) fn read(content: &[u8]) -> Result<Text<'_>, ReplaceSkip> {
    let encoding = if content.starts_with(UTF8_BOM) {
        Encoding::Utf8 { bom: true }
    } else if content.starts_with(&[0xFF, 0xFE]) {
        Encoding::Utf16 { big_endian: false }
    } else if content.starts_with(&[0xFE, 0xFF]) {
        Encoding::Utf16 { big_endian: true }
    } else if std::str::from_utf8(content).is_ok() {
        Encoding::Utf8 { bom: false }
    } else {
        Encoding::Ansi
    };
    let binary = match encoding {
        Encoding::Utf16 { .. } => false,
        _ => memchr::memchr(0, content).is_some(),
    };
    if binary {
        return Err(ReplaceSkip::Binary);
    }
    Ok(Text { content, encoding })
}

impl<'c> Text<'c> {
    /// Line 1 of a UTF-8 file with BOM: a hit may count the BOM's 3 bytes.
    pub(super) fn bom_shift(&self) -> bool {
        self.encoding == Encoding::Utf8 { bom: true }
    }

    /// The first haystack: the raw text (ANSI: its bytes), UTF-16 decoded.
    pub(super) fn haystack(&self) -> Result<Haystack<'c>, ReplaceSkip> {
        match self.encoding {
            Encoding::Utf8 { bom } => Ok(Haystack {
                bytes: Cow::Borrowed(&self.content[if bom { UTF8_BOM.len() } else { 0 }..]),
                domain: Domain::Utf8,
                transcoded: false,
            }),
            Encoding::Ansi => Ok(Haystack {
                bytes: Cow::Borrowed(self.content),
                domain: Domain::Ansi,
                transcoded: false,
            }),
            Encoding::Utf16 { big_endian } => {
                let codec = if big_endian {
                    encoding_rs::UTF_16BE
                } else {
                    encoding_rs::UTF_16LE
                };
                let text = codec
                    .decode_without_bom_handling_and_without_replacement(&self.content[2..])
                    .ok_or_else(|| ReplaceSkip::Unreadable("kein gültiges UTF-16".to_string()))?;
                if text.contains('\0') {
                    return Err(ReplaceSkip::Binary);
                }
                Ok(Haystack {
                    bytes: Cow::Owned(text.into_owned().into_bytes()),
                    domain: Domain::Utf8,
                    transcoded: true,
                })
            }
        }
    }

    /// An ANSI file decoded from Windows-1252 (the search's second pass for a
    /// pattern beyond ASCII); `None` for every other encoding.
    pub(super) fn transcoded(&self) -> Option<Result<Haystack<'c>, ReplaceSkip>> {
        if self.encoding != Encoding::Ansi {
            return None;
        }
        let mut text = String::new();
        if text.try_reserve(self.content.len() * 2).is_err() {
            return Some(Err(ReplaceSkip::TooLarge));
        }
        text.extend(self.content.iter().map(|byte| ansi_char(*byte)));
        Some(Ok(Haystack {
            bytes: Cow::Owned(text.into_bytes()),
            domain: Domain::Utf8,
            transcoded: true,
        }))
    }

    /// The new file from the new haystack bytes `bytes`, in the file's encoding.
    pub(super) fn encode(
        &self,
        haystack: &Haystack<'_>,
        bytes: Vec<u8>,
    ) -> Result<Vec<u8>, ReplaceSkip> {
        let text = |bytes: Vec<u8>| {
            String::from_utf8(bytes)
                .map_err(|_| ReplaceSkip::Unreadable("ungültiger Text nach dem Ersetzen".into()))
        };
        match (self.encoding, haystack.transcoded) {
            (Encoding::Utf8 { bom: true }, _) => {
                let mut out = Vec::new();
                out.try_reserve(bytes.len() + UTF8_BOM.len())
                    .map_err(|_| ReplaceSkip::TooLarge)?;
                out.extend_from_slice(UTF8_BOM);
                out.extend_from_slice(&bytes);
                Ok(out)
            }
            (Encoding::Utf8 { bom: false }, _) | (Encoding::Ansi, false) => Ok(bytes),
            (Encoding::Utf16 { big_endian }, _) => {
                let text = text(bytes)?;
                let mut out = Vec::new();
                out.try_reserve(text.len() * 2 + 2)
                    .map_err(|_| ReplaceSkip::TooLarge)?;
                let bom: [u8; 2] = if big_endian {
                    [0xFE, 0xFF]
                } else {
                    [0xFF, 0xFE]
                };
                out.extend_from_slice(&bom);
                for unit in text.encode_utf16() {
                    let pair = if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    };
                    out.extend_from_slice(&pair);
                }
                Ok(out)
            }
            (Encoding::Ansi, true) => {
                let text = text(bytes)?;
                let mut out = Vec::new();
                out.try_reserve(text.len())
                    .map_err(|_| ReplaceSkip::TooLarge)?;
                for character in text.chars() {
                    out.push(ansi_byte(character).ok_or(ReplaceSkip::NotAnsi)?);
                }
                Ok(out)
            }
        }
    }
}

/// Windows-1252 as `encoding_rs` decodes it (WHATWG: every byte has a
/// character, so decoding and encoding again give the same bytes).
fn ansi_table() -> &'static [char; 256] {
    static TABLE: OnceLock<[char; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = ['\0'; 256];
        for (byte, slot) in table.iter_mut().enumerate() {
            let (text, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(&[byte as u8]);
            *slot = text.chars().next().unwrap_or(char::REPLACEMENT_CHARACTER);
        }
        table
    })
}

/// The character of an ANSI byte.
pub(super) fn ansi_char(byte: u8) -> char {
    if byte.is_ascii() {
        char::from(byte)
    } else {
        ansi_table()[usize::from(byte)]
    }
}

/// The ANSI byte of a character; `None` where Windows-1252 has none.
pub(super) fn ansi_byte(character: char) -> Option<u8> {
    if character.is_ascii() {
        return Some(character as u8);
    }
    static REVERSE: OnceLock<Vec<(char, u8)>> = OnceLock::new();
    let reverse = REVERSE.get_or_init(|| {
        let mut pairs: Vec<(char, u8)> = (0x80..=0xFFu8)
            .map(|byte| (ansi_table()[usize::from(byte)], byte))
            .collect();
        pairs.sort_unstable();
        pairs
    });
    reverse
        .binary_search_by_key(&character, |(known, _)| *known)
        .ok()
        .map(|index| reverse[index].1)
}

/// `bytes` of a haystack as text for display.
pub(super) fn display(bytes: &[u8], domain: Domain) -> String {
    match domain {
        Domain::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        Domain::Ansi => bytes.iter().map(|byte| ansi_char(*byte)).collect(),
    }
}
