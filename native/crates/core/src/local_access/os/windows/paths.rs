//! Verbatim (`\\?\`) path handling for the Windows scanner.
//!
//! Win32 name resolution turns `C:\dir\nul` into the NUL *device*, strips
//! trailing dots and spaces, and refuses paths beyond 260 characters unless
//! the process is long-path aware. `GetFullPathNameW` — and therefore
//! `std::path::absolute` — applies the same device mapping, so the scanner
//! resolves roots itself: it only needs the current directory to anchor a
//! relative input. Every path is then opened in verbatim form, where names
//! are taken literally and the only limit is the 32 767-character NT
//! maximum. Children are joined with backslashes so the prefix keeps its
//! meaning; `.` and `..` are folded here because verbatim paths never are.
use std::path::{Path, PathBuf};

const VERBATIM: &str = r"\\?\";
const VERBATIM_UNC: &str = r"\\?\UNC\";
const DEVICE: &str = r"\\.\";

/// `\\?\C:\...` or `\\?\UNC\server\share\...` for any absolute or relative
/// input. A relative path that cannot be anchored (no current directory) is
/// returned unchanged so the open reports the real error instead of a
/// guessed one.
pub fn normalize_scan_root(root: &Path) -> PathBuf {
    let text = root.to_string_lossy().replace('/', "\\");
    if text.starts_with(VERBATIM) {
        // A bare `\\?\E:` or `\\?\Volume{GUID}` opens the volume device;
        // the folder meant is its root directory (U15).
        return PathBuf::from(verbatim_volume_root(&text).unwrap_or(text));
    }
    let current = || {
        std::env::current_dir()
            .map(|dir| dir.to_string_lossy().replace('/', "\\"))
            .ok()
    };
    match verbatim_from(&text, current) {
        Some(verbatim) => PathBuf::from(verbatim),
        None => PathBuf::from(text),
    }
}

/// `\\?\X:\` or `\\?\Volume{GUID}\` when `text` (either separator) is
/// exactly a verbatim volume root, with or without its final separator:
/// the root directory, which Win32 reaches only with that separator
/// (without it, the same name opens the volume device). `None` for every
/// other path, the device namespace (`\\.\`) included.
pub fn verbatim_volume_root(text: &str) -> Option<String> {
    let text = text.replace('/', "\\");
    let rest = text.strip_prefix(VERBATIM)?;
    let rest = rest.strip_suffix('\\').unwrap_or(rest);
    let bytes = rest.as_bytes();
    let drive = bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    (drive || is_volume_guid_name(rest)).then(|| format!("{VERBATIM}{rest}\\"))
}

/// `Volume{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`, the name the mount
/// manager gives every volume (case as the caller wrote it).
pub(super) fn is_volume_guid_name(name: &str) -> bool {
    let Some(inner) = name
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("Volume{"))
        .and_then(|_| name[7..].strip_suffix('}'))
    else {
        return false;
    };
    inner.len() == 36
        && inner.char_indices().all(|(index, unit)| match index {
            8 | 13 | 18 | 23 => unit == '-',
            _ => unit.is_ascii_hexdigit(),
        })
}

/// Resolve `text` (already backslash-separated, not verbatim) against the
/// current directory supplied by `current`, without any Win32 name mapping.
fn verbatim_from(text: &str, current: impl FnOnce() -> Option<String>) -> Option<String> {
    if let Some(rest) = text.strip_prefix(DEVICE) {
        // `\\.\C:\x` addresses the same object as `\\?\C:\x`; a bare device
        // (`\\.\nul`) stays a device, which the open then reports.
        return Some(format!("{VERBATIM}{}", fold_relative(rest)));
    }
    if let Some(rest) = text.strip_prefix(r"\\") {
        let (server_share, tail) = split_unc(rest);
        return Some(join_root(&format!("{VERBATIM_UNC}{server_share}"), tail));
    }
    let bytes = text.as_bytes();
    let drive_prefix = bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    if drive_prefix && bytes.get(2) == Some(&b'\\') {
        let drive = text[..2].to_ascii_uppercase();
        return Some(join_root(&format!("{VERBATIM}{drive}"), &text[3..]));
    }
    // Rootless (`\dir`), drive-relative (`C:dir`) and plain relative paths
    // are anchored on the current directory.
    let current = current()?;
    let (current_root, current_tail) = split_root(&current)?;
    if let Some(rest) = text.strip_prefix('\\') {
        return Some(join_root(&current_root, rest));
    }
    if drive_prefix {
        let drive = text[..2].to_ascii_uppercase();
        let root = format!("{VERBATIM}{drive}");
        if root.eq_ignore_ascii_case(&current_root) {
            return Some(join_root(&root, &format!("{current_tail}\\{}", &text[2..])));
        }
        return Some(join_root(&root, &text[2..]));
    }
    Some(join_root(&current_root, &format!("{current_tail}\\{text}")))
}

/// `\\?\C:` or `\\?\UNC\server\share` plus the remaining components of an
/// absolute current directory.
fn split_root(current: &str) -> Option<(String, String)> {
    let text = current.strip_prefix(VERBATIM).unwrap_or(current);
    if let Some(rest) = text
        .strip_prefix("UNC\\")
        .or_else(|| text.strip_prefix(r"\\"))
    {
        let (server_share, tail) = split_unc(rest);
        return Some((format!("{VERBATIM_UNC}{server_share}"), tail.to_string()));
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let tail = text[2..].trim_start_matches('\\');
        return Some((
            format!("{VERBATIM}{}", text[..2].to_ascii_uppercase()),
            tail.to_string(),
        ));
    }
    None
}

/// `server\share` and the remainder of a UNC body.
fn split_unc(rest: &str) -> (String, &str) {
    let mut parts = rest.splitn(3, '\\');
    let server = parts.next().unwrap_or("");
    let share = parts.next().unwrap_or("");
    let tail = parts.next().unwrap_or("");
    (format!("{server}\\{share}"), tail)
}

/// `root` plus the folded components of `tail`. A bare root keeps its
/// trailing separator (`\\?\C:\`); anything else drops it so joins never
/// produce a double backslash.
fn join_root(root: &str, tail: &str) -> String {
    let folded = fold_relative(tail);
    if folded.is_empty() {
        format!("{root}\\")
    } else {
        format!("{root}\\{folded}")
    }
}

/// Drop empty and `.` components, resolve `..` against the components
/// before it (never above the root), keep every other name literally —
/// including trailing dots and spaces, which Win32 would strip.
fn fold_relative(tail: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in tail.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    parts.join("\\")
}

/// Strip the verbatim prefix for messages and reports.
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(VERBATIM_UNC) {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(VERBATIM) {
        rest.to_string()
    } else {
        text.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(text: &str) -> String {
        verbatim_from(text, || Some(r"D:\work\here".to_string())).unwrap()
    }

    #[test]
    fn analytics_access_task_verbatim_roots_and_display() {
        assert_eq!(resolve(r"C:\"), r"\\?\C:\");
        assert_eq!(resolve(r"c:\Users\x\"), r"\\?\C:\Users\x");
        assert_eq!(resolve(r"C:\a\.\b\..\c\\d"), r"\\?\C:\a\c\d");
        assert_eq!(resolve(r"C:\..\..\x"), r"\\?\C:\x");
        assert_eq!(resolve(r"\\server\share\dir"), r"\\?\UNC\server\share\dir");
        assert_eq!(resolve(r"\\server\share"), r"\\?\UNC\server\share\");
        assert_eq!(resolve(r"\\.\C:\dev"), r"\\?\C:\dev");
        // Reserved device names and trailing dots/spaces stay literal names.
        assert_eq!(resolve(r"C:\data\nul"), r"\\?\C:\data\nul");
        assert_eq!(resolve(r"C:\data\trailing. "), r"\\?\C:\data\trailing. ");
        // Anchored on the supplied current directory.
        assert_eq!(resolve(r"sub\dir"), r"\\?\D:\work\here\sub\dir");
        assert_eq!(resolve(r"..\x"), r"\\?\D:\work\x");
        assert_eq!(resolve(r"\rooted"), r"\\?\D:\rooted");
        assert_eq!(resolve(r"D:rel"), r"\\?\D:\work\here\rel");
        assert_eq!(resolve(r"E:rel"), r"\\?\E:\rel");
        assert!(verbatim_from("relative", || None).is_none());
        assert_eq!(
            verbatim_from("x", || Some(r"\\srv\share\deep".to_string())).unwrap(),
            r"\\?\UNC\srv\share\deep\x"
        );

        assert_eq!(display_path(Path::new(r"\\?\C:\Users")), r"C:\Users");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\srv\share")),
            r"\\srv\share"
        );
        assert_eq!(display_path(Path::new(r"D:\plain")), r"D:\plain");

        let root = normalize_scan_root(Path::new("C:/"));
        assert_eq!(root.to_string_lossy(), r"\\?\C:\");
        let nul = normalize_scan_root(Path::new(r"C:\data\nul"));
        assert_eq!(nul.to_string_lossy(), r"\\?\C:\data\nul");
        assert_eq!(
            nul.join("child").to_string_lossy(),
            r"\\?\C:\data\nul\child"
        );
        let already = normalize_scan_root(Path::new(r"\\?\D:\kept\as is."));
        assert_eq!(already.to_string_lossy(), r"\\?\D:\kept\as is.");
    }

    #[test]
    fn removable_task_bare_verbatim_volume_is_its_root_folder() {
        let guid = "Volume{0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0}";
        for (input, root) in [
            (r"\\?\E:", r"\\?\E:\"),
            (r"\\?\e:\", r"\\?\e:\"),
            ("//?/E:", r"\\?\E:\"),
            ("//?/E:/", r"\\?\E:\"),
        ] {
            assert_eq!(
                verbatim_volume_root(input).as_deref(),
                Some(root),
                "{input}"
            );
        }
        let expected = format!(r"\\?\{guid}\");
        for input in [
            format!(r"\\?\{guid}"),
            format!(r"\\?\{guid}\"),
            format!("//?/{guid}"),
        ] {
            assert_eq!(
                verbatim_volume_root(&input),
                Some(expected.clone()),
                "{input}"
            );
            assert_eq!(
                normalize_scan_root(Path::new(&input)).to_string_lossy(),
                expected
            );
        }
        // Paths below a root, the device namespace, other verbatim forms and
        // malformed volume names stay as they are.
        for input in [
            r"\\?\E:\dir".to_string(),
            format!(r"\\?\{guid}\dir"),
            r"\\.\E:".to_string(),
            r"\\?\UNC\server\share".to_string(),
            r"\\?\GLOBALROOT\Device\HarddiskVolume3".to_string(),
            r"\\?\Volume{0b1c2d3e}".to_string(),
            r"\\?\Volume{0b1c2d3e-4f50-6172-8394-a5b6c7d8e9fz}".to_string(),
            r"E:".to_string(),
        ] {
            assert_eq!(verbatim_volume_root(&input), None, "{input}");
        }
        assert_eq!(
            normalize_scan_root(Path::new(r"\\?\E:")).to_string_lossy(),
            r"\\?\E:\"
        );
        assert_eq!(
            normalize_scan_root(Path::new(r"\\?\E:\dir")).to_string_lossy(),
            r"\\?\E:\dir"
        );
    }
}
