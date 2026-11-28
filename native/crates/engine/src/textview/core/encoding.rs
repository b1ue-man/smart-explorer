//! Character encoding of a text file, decided like Notepad++ does: a byte
//! order mark names UTF-8, UTF-16LE or UTF-16BE; without one the file is
//! UTF-8 when its bytes are valid UTF-8 and Windows-1252 ("ANSI") otherwise.
//! NUL bytes in the first 8 KiB mark a binary file, which the view refuses.
use serde::{Deserialize, Serialize};

use super::error::{TextViewError, TextViewResult};

/// Bytes of the file start that decide the encoding and the binary test.
pub const SNIFF_LEN: usize = 8 * 1024;

/// Encoding of a text document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextEncoding {
    /// UTF-8 without byte order mark (also plain ASCII).
    Utf8,
    /// UTF-8 with the byte order mark EF BB BF.
    Utf8Bom,
    /// UTF-16 little endian (byte order mark FF FE).
    Utf16Le,
    /// UTF-16 big endian (byte order mark FE FF).
    Utf16Be,
    /// Windows-1252, what Notepad++ calls "ANSI".
    Windows1252,
}

impl TextEncoding {
    /// Name the surfaces show in the status line.
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 mit BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Windows1252 => "Windows-1252 (ANSI)",
        }
    }

    /// Length of the byte order mark the text starts after.
    pub fn bom_len(self) -> u64 {
        match self {
            Self::Utf8Bom => 3,
            Self::Utf16Le | Self::Utf16Be => 2,
            Self::Utf8 | Self::Windows1252 => 0,
        }
    }

    /// Bytes of one code unit: 2 for UTF-16, 1 otherwise.
    pub fn unit_width(self) -> usize {
        match self {
            Self::Utf16Le | Self::Utf16Be => 2,
            _ => 1,
        }
    }

    /// Whether code units are big endian (UTF-16BE only).
    pub fn big_endian(self) -> bool {
        matches!(self, Self::Utf16Be)
    }

    /// Decodes the bytes of one line (without its line ending). Invalid
    /// sequences become U+FFFD, so decoding never fails.
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            Self::Utf8 | Self::Utf8Bom => String::from_utf8_lossy(bytes).into_owned(),
            Self::Utf16Le => encoding_rs::UTF_16LE
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
            Self::Utf16Be => encoding_rs::UTF_16BE
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
            Self::Windows1252 => encoding_rs::WINDOWS_1252
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
        }
    }
}

/// Decides the encoding from the first bytes of a file (at most
/// [`SNIFF_LEN`] are looked at). `whole_file` says that `prefix` is the whole
/// file, so a UTF-8 sequence cut at its end is invalid rather than
/// continued in the next bytes. Errors with [`TextViewError::NotText`] when
/// the prefix holds a NUL byte (UTF-16: a NUL code unit).
pub fn detect_encoding(prefix: &[u8], whole_file: bool) -> TextViewResult<TextEncoding> {
    let whole_file = whole_file && prefix.len() <= SNIFF_LEN;
    let prefix = &prefix[..prefix.len().min(SNIFF_LEN)];
    if prefix.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return no_nul_bytes(&prefix[3..]).map(|()| TextEncoding::Utf8Bom);
    }
    if prefix.starts_with(&[0xFF, 0xFE]) {
        return no_nul_units(&prefix[2..], false).map(|()| TextEncoding::Utf16Le);
    }
    if prefix.starts_with(&[0xFE, 0xFF]) {
        return no_nul_units(&prefix[2..], true).map(|()| TextEncoding::Utf16Be);
    }
    no_nul_bytes(prefix)?;
    let mut validator = Utf8Validator::default();
    validator.feed(prefix);
    let valid = if whole_file {
        validator.finish()
    } else {
        validator.is_valid()
    };
    Ok(if valid {
        TextEncoding::Utf8
    } else {
        TextEncoding::Windows1252
    })
}

fn no_nul_bytes(bytes: &[u8]) -> TextViewResult<()> {
    if memchr::memchr(0, bytes).is_some() {
        Err(TextViewError::NotText)
    } else {
        Ok(())
    }
}

fn no_nul_units(bytes: &[u8], big_endian: bool) -> TextViewResult<()> {
    let nul = bytes.chunks_exact(2).any(|unit| {
        let value = if big_endian {
            u16::from_be_bytes([unit[0], unit[1]])
        } else {
            u16::from_le_bytes([unit[0], unit[1]])
        };
        value == 0
    });
    if nul {
        Err(TextViewError::NotText)
    } else {
        Ok(())
    }
}

/// Checks UTF-8 validity of a byte stream fed in pieces: a sequence cut at a
/// piece boundary is completed by the next piece.
#[derive(Debug, Default)]
pub(crate) struct Utf8Validator {
    carry: Vec<u8>,
    invalid: bool,
}

impl Utf8Validator {
    pub(crate) fn feed(&mut self, mut bytes: &[u8]) {
        if self.invalid {
            return;
        }
        if !self.carry.is_empty() {
            // Complete the cut sequence with at most three more bytes.
            let take = bytes.len().min(4 - self.carry.len());
            self.carry.extend_from_slice(&bytes[..take]);
            match std::str::from_utf8(&self.carry) {
                Ok(_) => {
                    bytes = &bytes[take..];
                    self.carry.clear();
                }
                Err(error) if error.error_len().is_some() => {
                    self.invalid = true;
                    return;
                }
                Err(error) => {
                    let used = error.valid_up_to();
                    if used == 0 {
                        // Still incomplete: only possible when `bytes` ended.
                        if take == bytes.len() {
                            return;
                        }
                        self.invalid = true;
                        return;
                    }
                    // The cut sequence is complete; the rest belongs to `bytes`.
                    let rest = self.carry.len() - used;
                    bytes = &bytes[take - rest..];
                    self.carry.clear();
                }
            }
        }
        match std::str::from_utf8(bytes) {
            Ok(_) => {}
            Err(error) if error.error_len().is_some() => self.invalid = true,
            Err(error) => self.carry = bytes[error.valid_up_to()..].to_vec(),
        }
    }

    /// Valid so far (a cut sequence at the end still counts as valid).
    pub(crate) fn is_valid(&self) -> bool {
        !self.invalid
    }

    /// Valid as a whole stream: no cut sequence is left at its end.
    pub(crate) fn finish(&self) -> bool {
        !self.invalid && self.carry.is_empty()
    }
}
