/// Folder names that should never end up in the index. Covers:
///   - Windows system-reserved directories
///   - User-profile noise (AppData, Downloads - mostly cached/installer junk)
///   - Program/system roots (Program Files variants, ProgramData, Windows*)
///   - Dev caches (node_modules)
///
/// These get skipped whether they appear as a leaf folder or as any segment in
/// a longer path.
const SKIP_NAMES: &[&str] = &[
    // Pure system / recycle
    "$Recycle.Bin",
    "$RECYCLE.BIN",
    "System Volume Information",
    "$WinREAgent",
    "$SysReset",
    "Config.Msi",
    "MSOCache",
    "Recovery",
    "DumpStack.log.tmp",
    // User-profile heavyweight roots
    "AppData",
    "Downloads",
    // Program installs
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    // Windows itself
    "Windows",
    "Windows.old",
    "WinSxS",
    "PerfLogs",
    // Common dev cache (always noise, never navigation target)
    "node_modules",
];

/// Folder names that look auto-generated (hashes, UUIDs, build caches) and
/// shouldn't pollute the navigation index. Heuristics:
///   1. Pure-hex of length >= 8  (git hashes, npm/cargo cache keys, etc.)
///   2. UUID - 8-4-4-4-12 hex with dashes
///   3. Long base64-ish (>= 16 chars, only [A-Za-z0-9_-.=]) with very few
///      vowels (<12%), looks like an encoded ID rather than a word
pub fn is_generic_id(name: &str) -> bool {
    let n = name.len();
    if n < 8 {
        return false;
    }
    let bytes = name.as_bytes();

    // Rule 1: pure hex string of length >= 8
    let mut has_letter = false;
    let mut has_digit = false;
    let mut all_hex = true;
    for &b in bytes {
        match b {
            b'0'..=b'9' => has_digit = true,
            b'a'..=b'f' | b'A'..=b'F' => has_letter = true,
            _ => {
                all_hex = false;
                break;
            }
        }
    }
    if all_hex && has_letter && has_digit {
        return true;
    }
    // Pure-numeric of length >= 12 (probably an ID/timestamp folder)
    if n >= 12 && bytes.iter().all(|b| b.is_ascii_digit()) {
        return true;
    }

    // Rule 2: UUID 8-4-4-4-12 with hex digits
    if n == 36 && bytes[8] == b'-' && bytes[13] == b'-' && bytes[18] == b'-' && bytes[23] == b'-' {
        let only_hex_dash = bytes
            .iter()
            .all(|&b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' | b'-'));
        if only_hex_dash {
            return true;
        }
    }

    // Rule 3: long base64-ish with very few vowels
    if n >= 16 {
        let is_alnum_or_meta = bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'='));
        if is_alnum_or_meta {
            let n_vowels = bytes
                .iter()
                .filter(|&&b| {
                    matches!(
                        b,
                        b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U'
                    )
                })
                .count();
            if (n_vowels * 100) / n < 12 {
                return true;
            }
        }
    }

    false
}

/// True if this folder name should be excluded from the index.
pub fn should_skip(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    if SKIP_NAMES.iter().any(|s| s.eq_ignore_ascii_case(name)) {
        return true;
    }
    if is_generic_id(name) {
        return true;
    }
    false
}

/// True if any segment of `path` (separated by `/`) would be filtered out.
/// Used to clean legacy indices on load.
pub fn path_has_skipped_segment(path: &str) -> bool {
    path.split('/')
        .any(|seg| !seg.is_empty() && should_skip(seg))
}
