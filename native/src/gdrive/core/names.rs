//! Reversible, portable path segments for Drive's unrestricted object titles.
//! This is a namespace encoding, not URL encoding or a remote rename.
use std::io;

pub(super) fn encode(name: &str) -> String {
    let marker = super::duplicates::parse_marker(name).is_some();
    let stem = name.split('.').next().unwrap_or(name).to_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    let end = name
        .trim_end_matches(|c: char| c.is_whitespace() || c == '.')
        .len();
    let mut output = String::with_capacity(name.len());
    for (offset, c) in name.char_indices() {
        let escape = c.is_control()
            || matches!(
                c,
                '/' | '\\' | '%' | ':' | '"' | '<' | '>' | '|' | '?' | '*'
            )
            || (marker && c == '[')
            || (offset >= end && (c.is_whitespace() || c == '.'))
            || (offset == 0 && (c.is_whitespace() || device));
        if escape {
            let mut bytes = [0; 4];
            for byte in c.encode_utf8(&mut bytes).bytes() {
                output.push_str(&format!("%{byte:02X}"));
            }
        } else {
            output.push(c);
        }
    }
    output
}

pub(super) fn decode(segment: &str) -> io::Result<String> {
    validate_component(segment)?;
    // Recognize only the canonical encoding emitted by older versions. A
    // noncanonical percent sequence is a literal title, not malformed URI
    // input. New sync names always pass through sync_child_path/encode first.
    Ok(decode_canonical(segment).unwrap_or_else(|| segment.to_string()))
}

/// Provider-relative and logical sync components have no Windows path
/// semantics. Backslash, colon and whitespace remain literal title bytes;
/// physical target restrictions belong to the selected TargetLimits.
pub(super) fn validate_component(name: &str) -> io::Result<()> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains('/') || name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Drive name is not one path component",
        ));
    }
    Ok(())
}

fn decode_canonical(segment: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(segment.len());
    let input = segment.as_bytes();
    let mut offset = 0;
    while offset < input.len() {
        if input[offset] == b'%' {
            let hex = input.get(offset + 1..offset + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            offset += 3;
        } else {
            bytes.push(input[offset]);
            offset += 1;
        }
    }
    let name = String::from_utf8(bytes).ok()?;
    (encode(&name) == segment).then_some(name)
}
