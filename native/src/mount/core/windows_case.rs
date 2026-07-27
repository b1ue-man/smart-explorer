use std::io;

/// Portable equivalent of Windows ordinal ignore-case matching. Windows
/// compares case-insensitive path components through an invariant
/// one-code-point upcasing table ($UpCase / RtlUpcaseUnicodeChar) that never
/// expands a character: `ß` upcases to `ß`, so "Straße" and "STRASSE" are
/// distinct names even on case-insensitive volumes. Characters whose Unicode
/// uppercase mapping would expand to multiple code points therefore fold to
/// themselves, exactly like on NTFS, instead of being rejected — rejecting
/// them made every file or folder containing such a character (common German
/// names among them) invisible and unopenable through the mounted drive.
pub(crate) fn windows_ordinal_key(value: &str) -> String {
    value.chars().map(single_uppercase).collect()
}

fn single_uppercase(character: char) -> char {
    let mut uppercase = character.to_uppercase();
    let first = uppercase.next().unwrap_or(character);
    if uppercase.next().is_none() {
        first
    } else {
        // Multi-code-point expansions do not exist in the Windows ordinal
        // upcase table; such characters are their own case class there.
        character
    }
}

/// Every character now has a total, unambiguous single-code-point fold, so
/// no component spelling needs to be rejected for case-identity safety. The
/// validator is kept as the single documented seam should a future backend
/// class ever require narrowing again.
pub(crate) fn validate_windows_case_component(_component: &str) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_ordinal_key_matches_windows_single_char_upcasing() {
        assert_eq!(windows_ordinal_key("readme.txt"), "README.TXT");
        assert_eq!(windows_ordinal_key("Grüße"), "GRÜßE");
        // ß never expands to SS: the two spellings stay distinct names.
        assert_ne!(windows_ordinal_key("Straße"), windows_ordinal_key("STRASSE"));
        // Case-only variants of the same sharp-s name still match.
        assert_eq!(windows_ordinal_key("straße"), windows_ordinal_key("STRAßE"));
    }

    #[test]
    fn remote_drive_task_expanding_uppercase_components_are_representable() {
        for name in ["Straße", "Maße 2026.pdf", "ﬁle", "ŉou"] {
            assert!(validate_windows_case_component(name).is_ok());
            let key = windows_ordinal_key(name);
            assert_eq!(key.chars().count(), name.chars().count());
            assert!(!key.contains('\0'));
        }
    }
}
