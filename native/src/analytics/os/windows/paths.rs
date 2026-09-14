//! Verbatim (`\\?\`) path handling for the Windows scanner.
//!
//! Win32 name resolution turns `C:\dir\nul` into the NUL *device*, strips
//! trailing dots and spaces, and refuses paths beyond 260 characters unless
//! the process is long-path aware. The scanner therefore opens every path in
//! verbatim form, where names are taken literally and the only limit is the
//! 32 767-character NT maximum. Children are joined with backslashes so the
//! verbatim prefix keeps its meaning.
use std::path::{Path, PathBuf};

const VERBATIM: &str = r"\\?\";
const VERBATIM_UNC: &str = r"\\?\UNC\";

/// `\\?\C:\...` or `\\?\UNC\server\share\...` for any absolute or relative
/// input. A path that cannot be made absolute is returned unchanged so the
/// open reports the real error instead of a guessed one.
pub(in crate::analytics::os) fn normalize_scan_root(root: &Path) -> PathBuf {
    let text = root.to_string_lossy().replace('/', "\\");
    if text.starts_with(VERBATIM) {
        return PathBuf::from(text);
    }
    let absolute = match std::path::absolute(Path::new(&text)) {
        Ok(absolute) => absolute.to_string_lossy().replace('/', "\\"),
        Err(_) => return PathBuf::from(text),
    };
    PathBuf::from(verbatim_from_absolute(&absolute))
}

fn verbatim_from_absolute(absolute: &str) -> String {
    if absolute.starts_with(VERBATIM) {
        return absolute.to_string();
    }
    if let Some(rest) = absolute.strip_prefix(r"\\.\") {
        return format!("{VERBATIM}{rest}");
    }
    if let Some(rest) = absolute.strip_prefix(r"\\") {
        return format!("{VERBATIM_UNC}{rest}");
    }
    let mut text = absolute.to_string();
    // A drive root keeps its trailing separator; anything else drops it so
    // joins never produce a double backslash.
    if text.len() > 3 {
        while text.ends_with('\\') {
            text.pop();
        }
    }
    format!("{VERBATIM}{text}")
}

/// Strip the verbatim prefix for messages and reports.
pub(in crate::analytics::os) fn display_path(path: &Path) -> String {
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

    #[test]
    fn analytics_access_task_verbatim_roots_and_display() {
        assert_eq!(verbatim_from_absolute(r"C:\"), r"\\?\C:\");
        assert_eq!(verbatim_from_absolute(r"C:\Users\x\"), r"\\?\C:\Users\x");
        assert_eq!(verbatim_from_absolute(r"\\server\share\dir"), r"\\?\UNC\server\share\dir");
        assert_eq!(verbatim_from_absolute(r"\\?\D:\already"), r"\\?\D:\already");
        assert_eq!(display_path(Path::new(r"\\?\C:\Users")), r"C:\Users");
        assert_eq!(display_path(Path::new(r"\\?\UNC\srv\share")), r"\\srv\share");
        assert_eq!(display_path(Path::new(r"D:\plain")), r"D:\plain");
        let root = normalize_scan_root(Path::new("C:/"));
        assert_eq!(root.to_string_lossy(), r"\\?\C:\");
        let nul = normalize_scan_root(Path::new(r"C:\data\nul"));
        assert_eq!(nul.to_string_lossy(), r"\\?\C:\data\nul");
        assert_eq!(
            nul.join("child").to_string_lossy(),
            r"\\?\C:\data\nul\child"
        );
    }
}
