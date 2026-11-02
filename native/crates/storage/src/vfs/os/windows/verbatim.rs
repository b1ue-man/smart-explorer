//! Verbatim (`\\?\`) paths for names Win32 would not address literally.
//!
//! Win32 name resolution maps `C:\dir\NUL` (and `nul.txt`, `con.log`, …) to
//! the device, strips trailing dots and spaces, and rejects a few characters.
//! Rust's std hands paths shorter than 248 UTF-16 units to Win32 unchanged, so
//! the local backend has to build the verbatim form itself when a component
//! needs it. Verbatim paths skip that resolution but also never fold `.` and
//! `..`, so those are folded here and separators must be backslashes.
use crate::types::win32_name_issue;

const VERBATIM: &str = r"\\?\";
const NT_PREFIX: &str = r"\??\";
const VERBATIM_UNC: &str = r"\\?\UNC\";

/// `Some(verbatim)` when `native` (backslash-separated) is an absolute drive
/// or UNC path with at least one component Win32 would not address literally;
/// `None` when the plain spelling is safe, the path is already verbatim, or
/// it is not absolute (then no verbatim form can be derived safely).
pub(super) fn verbatim_if_hostile(native: &str) -> Option<String> {
    if native.starts_with(VERBATIM) || native.starts_with(NT_PREFIX) {
        return None;
    }
    let (root, tail) = split_root(native)?;
    let components: Vec<&str> = tail
        .split('\\')
        .filter(|component| !component.is_empty())
        .collect();
    if !components
        .iter()
        .any(|component| win32_name_issue(component).is_some())
    {
        return None;
    }
    let mut folded: Vec<&str> = Vec::new();
    for component in components {
        match component {
            "." => {}
            ".." => {
                folded.pop();
            }
            name => folded.push(name),
        }
    }
    if folded.is_empty() {
        Some(format!("{root}\\"))
    } else {
        Some(format!("{root}\\{}", folded.join("\\")))
    }
}

/// `\\?\C:` or `\\?\UNC\server\share` plus the remaining text, for absolute
/// drive-rooted and UNC inputs only. Device paths (`\\.\`) are left alone.
fn split_root(native: &str) -> Option<(String, &str)> {
    if let Some(rest) = native.strip_prefix(r"\\") {
        let mut parts = rest.splitn(3, '\\');
        let server = parts.next()?;
        let share = parts.next()?;
        if server.is_empty() || share.is_empty() || matches!(server, "." | "?") {
            return None;
        }
        let tail = parts.next().unwrap_or("");
        return Some((format!("{VERBATIM_UNC}{server}\\{share}"), tail));
    }
    let bytes = native.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        let drive = native[..2].to_ascii_uppercase();
        return Some((format!("{VERBATIM}{drive}"), &native[3..]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{start_scan, ScanMessage, ScanOpts};
    use crate::vfs::{remove_entry, Backend, DeleteTarget, LocalBackend};
    use std::path::{Path, PathBuf};

    #[test]
    fn recursive_filter_task_verbatim_only_for_hostile_components() {
        assert_eq!(
            verbatim_if_hostile(r"C:\data\nul").as_deref(),
            Some(r"\\?\C:\data\nul")
        );
        assert_eq!(
            verbatim_if_hostile(r"c:\data\NUL.txt").as_deref(),
            Some(r"\\?\C:\data\NUL.txt")
        );
        assert_eq!(
            verbatim_if_hostile(r"C:\data\report.").as_deref(),
            Some(r"\\?\C:\data\report.")
        );
        assert_eq!(
            verbatim_if_hostile(r"C:\aux \child.txt").as_deref(),
            Some(r"\\?\C:\aux \child.txt")
        );
        assert_eq!(
            verbatim_if_hostile(r"C:\a\.\b\..\nul").as_deref(),
            Some(r"\\?\C:\a\nul")
        );
        assert_eq!(
            verbatim_if_hostile(r"\\server\share\dir\con").as_deref(),
            Some(r"\\?\UNC\server\share\dir\con")
        );
        assert_eq!(verbatim_if_hostile(r"C:\data\plain.txt"), None);
        assert_eq!(verbatim_if_hostile(r"C:\"), None);
        assert_eq!(verbatim_if_hostile(r"\\server\share\plain"), None);
        assert_eq!(verbatim_if_hostile(r"\\?\C:\data\nul"), None);
        assert_eq!(verbatim_if_hostile(r"\\.\C:\nul"), None);
        assert_eq!(verbatim_if_hostile(r"relative\nul"), None);
    }

    fn temp_root(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("se_hostile_{label}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn verbatim(path: &Path) -> PathBuf {
        let text = path.to_string_lossy().replace('/', "\\");
        PathBuf::from(verbatim_if_hostile(&text).unwrap_or(text))
    }

    fn forward(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn recursive_filter_task_local_backend_addresses_real_hostile_names() {
        let root = temp_root("vfs");
        let nul = root.join("nul");
        let trailing = root.join("report.");
        std::fs::write(verbatim(&nul), b"abc").unwrap();
        std::fs::write(verbatim(&trailing), b"de").unwrap();
        std::fs::write(root.join("plain.txt"), b"f").unwrap();

        let backend = LocalBackend::new("/");
        let listed = backend.list_dir(&forward(&root)).unwrap();
        let sizes: std::collections::HashMap<String, u64> = listed
            .iter()
            .map(|meta| (meta.name.clone(), meta.size))
            .collect();
        assert_eq!(sizes.get("nul"), Some(&3), "{sizes:?}");
        assert_eq!(sizes.get("report."), Some(&2), "{sizes:?}");
        assert_eq!(sizes.get("plain.txt"), Some(&1));

        let stat = backend.stat(&forward(&nul)).unwrap();
        assert!(!stat.is_dir && stat.size == 3, "{stat:?}");
        assert_eq!(stat.name, "nul");

        let renamed = root.join("_nul.txt");
        backend
            .rename_no_replace(&forward(&nul), &forward(&renamed))
            .unwrap();
        assert_eq!(std::fs::read(&renamed).unwrap(), b"abc");
        assert!(std::fs::symlink_metadata(verbatim(&nul)).is_err());

        std::fs::write(verbatim(&nul), b"abc").unwrap();
        for path in [&nul, &trailing] {
            let target = DeleteTarget {
                path: forward(path),
                id: None,
                is_dir: false,
                is_symlink: false,
            };
            remove_entry(&backend, &target).unwrap();
            assert!(std::fs::symlink_metadata(verbatim(path)).is_err());
        }

        let hostile_dir = root.join("aux");
        std::fs::create_dir(verbatim(&hostile_dir)).unwrap();
        std::fs::write(verbatim(&hostile_dir.join("inner.txt")), b"x").unwrap();
        let target = DeleteTarget {
            path: forward(&hostile_dir),
            id: None,
            is_dir: true,
            is_symlink: false,
        };
        remove_entry(&backend, &target).unwrap();
        assert!(std::fs::symlink_metadata(verbatim(&hostile_dir)).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recursive_filter_task_scanner_lists_real_hostile_names_with_metadata() {
        let root = temp_root("scan");
        std::fs::write(verbatim(&root.join("nul")), b"abcd").unwrap();
        std::fs::create_dir(verbatim(&root.join("con"))).unwrap();
        std::fs::write(verbatim(&root.join("con").join("inner.")), b"xy").unwrap();

        let (tx, rx) = crossbeam_channel::unbounded();
        start_scan(root.clone(), ScanOpts::everything(None), tx);
        let mut entries = Vec::new();
        let mut errors = 0;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
                ScanMessage::Entries(batch) => entries.extend(batch),
                ScanMessage::Done(progress) => {
                    errors = progress.errors;
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(errors, 0);
        let nul = entries
            .iter()
            .find(|entry| entry.name.as_ref() == "nul")
            .expect("nul file listed");
        assert!(!nul.is_dir && nul.size == 4, "{nul:?}");
        let con = entries
            .iter()
            .find(|entry| entry.name.as_ref() == "con")
            .expect("con directory listed");
        assert!(con.is_dir);
        let inner = entries
            .iter()
            .find(|entry| entry.name.as_ref() == "inner.")
            .expect("inner. listed below con");
        assert_eq!(inner.size, 2);

        let outcome = crate::scanner::collect_recursive(
            &root,
            false,
            1,
            &std::sync::atomic::AtomicBool::new(false),
        );
        assert!(outcome.is_complete(), "{:?}", outcome.issues);
        assert_eq!(outcome.entries.len(), 3);
        std::fs::remove_dir_all(verbatim(&root)).ok();
    }
}
