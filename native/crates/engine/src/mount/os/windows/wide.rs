use std::{io, slice};

const MAX_WINDOWS_PATH_UNITS: usize = 32_767;
const MAX_FIND_NAME_UNITS: usize = 259;

pub(super) unsafe fn read_wide(value: *const u16) -> io::Result<String> {
    if value.is_null() {
        return Err(invalid("null UTF-16 string"));
    }
    let mut length = 0usize;
    while length <= MAX_WINDOWS_PATH_UNITS {
        if unsafe { *value.add(length) } == 0 {
            let units = unsafe { slice::from_raw_parts(value, length) };
            return String::from_utf16(units).map_err(|_| invalid("invalid UTF-16 string"));
        }
        length += 1;
    }
    Err(invalid("unterminated or overlong UTF-16 string"))
}

pub(super) fn encode_mount_point(letter: char) -> Vec<u16> {
    format!("{}:\\", letter.to_ascii_uppercase())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

pub(super) fn encode_find_name(name: &str) -> io::Result<[u16; 260]> {
    validate_component(name)?;
    let encoded = name.encode_utf16().collect::<Vec<_>>();
    if encoded.len() > MAX_FIND_NAME_UNITS {
        return Err(invalid("file name is too long for WIN32_FIND_DATAW"));
    }
    let mut destination = [0u16; 260];
    destination[..encoded.len()].copy_from_slice(&encoded);
    Ok(destination)
}

pub(super) unsafe fn write_wide(
    destination: *mut u16,
    capacity: u32,
    value: &str,
    maximum_units: Option<usize>,
) -> io::Result<()> {
    if destination.is_null() || capacity == 0 {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "UTF-16 output buffer is missing",
        ));
    }
    let limit = maximum_units.unwrap_or(usize::MAX);
    let mut encoded = Vec::new();
    for character in value.chars() {
        let mut units = [0u16; 2];
        let next = character.encode_utf16(&mut units);
        if encoded.len() + next.len() > limit {
            break;
        }
        encoded.extend_from_slice(next);
    }
    if encoded.len() + 1 > capacity as usize {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "UTF-16 output buffer is too small",
        ));
    }
    unsafe {
        std::ptr::write_bytes(destination, 0, capacity as usize);
        std::ptr::copy_nonoverlapping(encoded.as_ptr(), destination, encoded.len());
    }
    Ok(())
}

fn validate_component(name: &str) -> io::Result<()> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.ends_with(' ')
        || name.ends_with('.')
        || name.chars().any(|character| {
            character < '\u{20}'
                || matches!(
                    character,
                    '\0' | '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
        || is_reserved_device_name(name)
    {
        return Err(invalid("remote entry has no safe Windows file name"));
    }
    Ok(())
}

fn is_reserved_device_name(name: &str) -> bool {
    let stem = name
        .split_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(name)
        .trim_end_matches(|character| matches!(character, ' ' | '.'));
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
