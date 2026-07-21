use std::io;

/// Portable subset of Windows ordinal ignore-case matching. Windows compares
/// case-insensitive path components through invariant one-code-point upcasing;
/// Unicode mappings that expand to multiple code points are rejected instead
/// of being guessed differently in the daemon, cache, and callback layers.
pub(crate) fn windows_ordinal_key(value: &str) -> String {
    let mut key = String::with_capacity(value.len());
    for character in value.chars() {
        let mut uppercase = character.to_uppercase();
        let first = uppercase.next().unwrap_or(character);
        if uppercase.next().is_none() {
            key.push(first);
        } else {
            // Valid mounted components never take this branch. Retaining a
            // tagged original makes unvalidated bookkeeping fail closed.
            key.push('\0');
            key.push(character);
        }
    }
    key
}

pub(crate) fn validate_windows_case_component(component: &str) -> io::Result<()> {
    if component
        .chars()
        .any(|character| character.to_uppercase().count() != 1)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path component has no unambiguous Windows ordinal case mapping",
        ));
    }
    Ok(())
}
