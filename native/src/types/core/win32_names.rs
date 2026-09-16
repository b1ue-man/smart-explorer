//! Win32 file-name rules that make a stored name unaddressable through an
//! ordinary Win32 path. Pure string logic shared by the filter (to select such
//! names), the local VFS adapter (to decide when a verbatim `\\?\` path is
//! required) and the delete flow (to pick a safe name before recycling).
//!
//! Win32 name resolution (`RtlIsDosDeviceName_U`) inspects the final path
//! component after removing trailing dots and spaces and the text from the
//! first period on, so `NUL`, `nul.txt` and `NUL .tar.gz` all address the NUL
//! *device*; trailing dots and spaces are stripped, so `report.` can never be
//! opened as `report.`. Only a verbatim path reaches the stored name.

/// Why Win32 name resolution would not address a stored name literally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Win32NameIssue {
    /// `NUL`, `CON`, `PRN`, `AUX`, `COM0-9`, `LPT0-9`, `COM¹²³`, `LPT¹²³`, with
    /// or without an extension: Win32 opens the device instead of the file.
    ReservedDevice,
    /// Win32 strips trailing dots and spaces before resolving the name.
    TrailingDotOrSpace,
    /// `< > : " | ? * \ /` or control characters are rejected by Win32.
    InvalidCharacter,
}

impl Win32NameIssue {
    /// Short German description for tooltips and messages.
    pub fn label_de(self) -> &'static str {
        match self {
            Self::ReservedDevice => "reservierter Windows-Gerätename",
            Self::TrailingDotOrSpace => "endet mit Punkt oder Leerzeichen",
            Self::InvalidCharacter => "enthält unter Windows ungültige Zeichen",
        }
    }
}

/// The first reason a stored name cannot be addressed by an ordinary Win32
/// path, or `None` for a plain name. `.` and `..` are never stored names.
pub fn win32_name_issue(name: &str) -> Option<Win32NameIssue> {
    if name.is_empty() || matches!(name, "." | "..") {
        return None;
    }
    if name.chars().any(is_win32_invalid_char) {
        return Some(Win32NameIssue::InvalidCharacter);
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some(Win32NameIssue::TrailingDotOrSpace);
    }
    if is_dos_device_name(name) {
        return Some(Win32NameIssue::ReservedDevice);
    }
    None
}

/// True when Win32 would address `name` as a device rather than a file.
pub fn is_win32_device_name(name: &str) -> bool {
    is_dos_device_name(name)
}

/// A sibling name Win32 can address: invalid characters become `_`, trailing
/// dots and spaces are removed, a reserved device stem gets a `_` prefix and
/// an otherwise empty result becomes `unbenannt`. The caller still has to make
/// the result unique within its folder.
pub fn win32_safe_name(name: &str) -> String {
    let mut safe: String = name
        .chars()
        .map(|character| {
            if is_win32_invalid_char(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let kept = safe.trim_end_matches([' ', '.']).len();
    safe.truncate(kept);
    if safe.is_empty() {
        return "unbenannt".to_string();
    }
    if is_dos_device_name(&safe) {
        safe.insert(0, '_');
    }
    safe
}

fn is_win32_invalid_char(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '/'
        )
}

/// `RtlIsDosDeviceName_U` on one component: ignore trailing dots and spaces,
/// cut the text from the first period (an "extension"), ignore spaces before
/// it, then compare the remaining stem with the device names.
fn is_dos_device_name(name: &str) -> bool {
    let trimmed = name.trim_end_matches([' ', '.']);
    let stem_end = trimmed.find('.').unwrap_or(trimmed.len());
    let stem = trimmed[..stem_end].trim_end_matches(' ');
    let mut letters = stem.chars().map(|character| character.to_ascii_uppercase());
    let (Some(first), Some(second), Some(third)) = (letters.next(), letters.next(), letters.next())
    else {
        return false;
    };
    let head = (first, second, third);
    match (letters.next(), letters.next()) {
        (None, _) => matches!(
            head,
            ('C', 'O', 'N') | ('P', 'R', 'N') | ('A', 'U', 'X') | ('N', 'U', 'L')
        ),
        (Some(digit), None) => {
            matches!(head, ('C', 'O', 'M') | ('L', 'P', 'T'))
                && matches!(digit, '0'..='9' | '¹' | '²' | '³')
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_filter_task_reserved_device_names_are_detected() {
        for name in [
            "nul",
            "NUL",
            "Nul.txt",
            "con.tar.gz",
            "AUX",
            "prn",
            "com1",
            "COM9",
            "LPT0",
            "lpt¹",
            "COM³.log",
            "nul  .txt",
            "nul.",
            "aux ",
        ] {
            assert!(
                matches!(
                    win32_name_issue(name),
                    Some(Win32NameIssue::ReservedDevice | Win32NameIssue::TrailingDotOrSpace)
                ),
                "{name:?} was not flagged"
            );
            assert!(
                is_win32_device_name(name.trim_end_matches([' ', '.'])),
                "{name:?} is a device stem"
            );
        }
    }

    #[test]
    fn recursive_filter_task_trailing_and_invalid_characters_are_detected() {
        assert_eq!(
            win32_name_issue("report."),
            Some(Win32NameIssue::TrailingDotOrSpace)
        );
        assert_eq!(
            win32_name_issue("report "),
            Some(Win32NameIssue::TrailingDotOrSpace)
        );
        for name in [
            "a<b", "a>b", "a:b", "a\"b", "a|b", "a?b", "a*b", "a\\b", "a/b", "a\tb",
        ] {
            assert_eq!(
                win32_name_issue(name),
                Some(Win32NameIssue::InvalidCharacter),
                "{name:?}"
            );
        }
    }

    #[test]
    fn recursive_filter_task_ordinary_names_pass() {
        for name in [
            "null",
            "com",
            "com10",
            "lpt",
            "nul_x",
            "nulled.txt",
            "con-cat",
            "data.bin",
            ".gitignore",
            "a.b.c",
            "Ärger.txt",
            "COMPUTER",
            "AUXILIARY",
            ".",
            "..",
            "",
        ] {
            assert_eq!(win32_name_issue(name), None, "{name:?} was flagged");
        }
        assert!(!is_win32_device_name("com10"));
        assert!(!is_win32_device_name("nul_x"));
    }

    #[test]
    fn recursive_filter_task_safe_names_are_addressable() {
        assert_eq!(win32_safe_name("nul.txt"), "_nul.txt");
        assert_eq!(win32_safe_name("NUL"), "_NUL");
        assert_eq!(win32_safe_name("report. "), "report");
        assert_eq!(win32_safe_name("a<b>c"), "a_b_c");
        assert_eq!(win32_safe_name("..."), "unbenannt");
        assert_eq!(win32_safe_name("plain.txt"), "plain.txt");
        for name in [
            "nul.txt", "NUL", "report. ", "a<b>c", "...", "com1", "aux .log",
        ] {
            assert_eq!(win32_name_issue(&win32_safe_name(name)), None, "{name:?}");
        }
    }
}
