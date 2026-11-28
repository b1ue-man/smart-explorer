//! Placeholders `⟦Label-N⟧` and `⟦Label-N.V⟧` (plan S2): U+27E6/U+27E7 close
//! the placeholder, so `⟦Name-1⟧` is never a prefix of `⟦Name-10⟧`
//! (reference §3.4). A masked match over k line endings carries the same
//! line endings before `⟧`, so line numbers stay the same. A `⟦` of the
//! original text is written as `⟦⟦`.

/// Opening character of a placeholder.
pub const PLACEHOLDER_OPEN: char = '⟦';
/// Closing character of a placeholder.
pub const PLACEHOLDER_CLOSE: char = '⟧';

pub(crate) const OPEN: &[u8] = "⟦".as_bytes();
pub(crate) const CLOSE: &[u8] = "⟧".as_bytes();

/// A placeholder read from an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub label: String,
    pub number: u64,
    pub variant: Option<u64>,
    /// Bytes from `⟦` to `⟧` inclusive.
    pub len: usize,
}

/// Writes `⟦label-number[.variant]<line endings>⟧`.
pub(crate) fn write(
    label: &str,
    number: u64,
    variant: Option<u64>,
    line_endings: &[u8],
    out: &mut Vec<u8>,
) {
    out.extend_from_slice(OPEN);
    out.extend_from_slice(label.as_bytes());
    out.push(b'-');
    out.extend_from_slice(number.to_string().as_bytes());
    if let Some(variant) = variant {
        out.push(b'.');
        out.extend_from_slice(variant.to_string().as_bytes());
    }
    out.extend_from_slice(line_endings);
    out.extend_from_slice(CLOSE);
}

/// The line endings of `bytes` in order (`\r\n`, `\n`, `\r`).
pub(crate) fn line_endings(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .copied()
        .filter(|b| *b == b'\r' || *b == b'\n')
        .collect()
}

/// Reads a placeholder that starts at `pos` (at `⟦`).
pub(crate) fn parse_at(bytes: &[u8], pos: usize) -> Option<Parsed> {
    let rest = bytes.get(pos..)?.strip_prefix(OPEN)?;
    let text = utf8_prefix(rest);
    let mut chars = text.char_indices().peekable();
    let mut label = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_alphanumeric() || c == '_' {
            label.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if label.is_empty() || label.chars().count() > super::model::MAX_LABEL_CHARS {
        return None;
    }
    if chars.next()?.1 != '-' {
        return None;
    }
    let number = digits(&mut chars)?;
    let variant = if chars.peek().map(|(_, c)| *c) == Some('.') {
        chars.next();
        Some(digits(&mut chars)?)
    } else {
        None
    };
    // The line endings may be many (a long multi-line match): read them
    // from the bytes, not from the decoded window.
    let mut index = chars.peek().map_or(text.len(), |(offset, _)| *offset);
    while matches!(rest.get(index), Some(b'\r' | b'\n')) {
        index += 1;
    }
    if !rest.get(index..)?.starts_with(CLOSE) {
        return None;
    }
    Some(Parsed {
        label,
        number,
        variant,
        len: OPEN.len() + index + CLOSE.len(),
    })
}

fn digits(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> Option<u64> {
    let mut text = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_ascii_digit() && text.len() < 19 {
            text.push(c);
            chars.next();
        } else {
            break;
        }
    }
    text.parse().ok()
}

/// The longest valid UTF-8 prefix (a placeholder is short and valid text).
fn utf8_prefix(bytes: &[u8]) -> &str {
    let window = &bytes[..bytes.len().min(256)];
    match std::str::from_utf8(window) {
        Ok(text) => text,
        Err(error) => std::str::from_utf8(&window[..error.valid_up_to()]).unwrap_or_default(),
    }
}

/// Whether `text` is exactly one placeholder (surfaces that want to tell a
/// masked name from a real one).
pub fn is_placeholder_text(text: &str) -> bool {
    parse_at(text.as_bytes(), 0).is_some_and(|parsed| parsed.len == text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_task_placeholders_parse_with_variants_and_line_endings() {
        let mut out = Vec::new();
        write("Adresse", 12, Some(3), b"\r\n\n", &mut out);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, "⟦Adresse-12.3\r\n\n⟧");
        let parsed = parse_at(text.as_bytes(), 0).unwrap();
        assert_eq!(
            parsed,
            Parsed {
                label: "Adresse".into(),
                number: 12,
                variant: Some(3),
                len: text.len()
            }
        );
        assert!(is_placeholder_text("⟦Maske-1⟧"));
        for bad in [
            "⟦Maske-⟧",
            "⟦-1⟧",
            "⟦Maske1⟧",
            "⟦Maske-1",
            "⟦Ma ske-1⟧",
            "⟦Maske-1.⟧",
        ] {
            assert!(parse_at(bad.as_bytes(), 0).is_none(), "{bad}");
        }
        assert_eq!(line_endings(b"a\r\nb\nc"), b"\r\n\n");
    }
}
