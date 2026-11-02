//! `se ls --recursive`: the explorer's recursive view (Strg+R) in the
//! terminal. It walks with the same scanners as the view, `scanner` on a
//! local folder and `rscan` on a remote one (one request for the whole tree
//! where the other side offers it), and prints every entry below the root.
use super::target::Target;
use crate::scanner::{ScanMessage, ScanOpts};
use crate::types::FileEntry;

/// Prints the tree below `target`; unreadable paths go to stderr and make the
/// exit status non-zero, entries read before them stay printed.
pub(super) fn list(target: &Target, json: bool) -> Result<i32, String> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let handle = if target.backend.is_local() {
        crate::scanner::start_scan(
            std::path::PathBuf::from(&target.path),
            ScanOpts {
                follow_symlinks: false,
                max_depth: None,
                retention: None,
            },
            tx,
        )
    } else {
        crate::rscan::start_scan_backend(
            target.backend.clone(),
            target.path.clone(),
            None,
            None,
            tx,
        )
    };
    // The scan's own text for the root (its depth-0 entry, sent first). Every
    // entry below it carries that text as its prefix, whatever form the scanner
    // gave the root (verbatim on Windows, a relative target made absolute).
    let mut root: Option<String> = None;
    let mut rows = Vec::new();
    let mut failed = Vec::new();
    let mut error = None;
    for message in rx {
        match message {
            ScanMessage::Entries(entries) => {
                for entry in entries {
                    if entry.depth == 0 {
                        if root.is_none() {
                            root = Some(entry.path.to_string());
                        }
                    } else if json {
                        rows.push(row(root.as_deref(), &entry));
                    } else {
                        print_line(root.as_deref(), &entry);
                    }
                }
            }
            ScanMessage::FailedPaths(paths) => failed.extend(paths),
            ScanMessage::Error(text) => error = Some(text),
            ScanMessage::Progress(_) | ScanMessage::DeniedRoots(_) => {}
            ScanMessage::Done(_) => break,
        }
    }
    if json {
        let text = serde_json::to_string_pretty(&rows).map_err(|e| format!("JSON: {e}"))?;
        println!("{text}");
    }
    for (path, reason) in &failed {
        eprintln!("se: ls: not read: {path}: {reason}");
    }
    let truncated = handle.truncated.load(std::sync::atomic::Ordering::Relaxed);
    if truncated {
        eprintln!("se: ls: stopped at the scan limit; the listing is incomplete");
    }
    if let Some(error) = error {
        return Err(format!("ls {}: {error}", target.path));
    }
    Ok(if failed.is_empty() && !truncated {
        0
    } else {
        1
    })
}

/// The path of `entry` below the listed folder, with `/` between the components
/// on every platform. `root` is the scan's text for that folder; an entry that
/// does not lie below it keeps its full path.
fn relative(root: Option<&str>, entry: &FileEntry) -> String {
    let path: &str = &entry.path;
    let below = root
        .map(|root| root.trim_end_matches('/'))
        .and_then(|root| path.strip_prefix(root))
        .filter(|rest| rest.starts_with('/'))
        .map(|rest| rest.trim_start_matches('/'))
        .filter(|rest| !rest.is_empty());
    match below {
        Some(rest) => slashes(rest),
        None => path.to_string(),
    }
}

/// A name cannot contain `\` on Windows, so every separator there is `/`.
#[cfg(windows)]
fn slashes(text: &str) -> String {
    text.replace('\\', "/")
}

/// On other platforms `\` is an ordinary name character and stays as it is.
#[cfg(not(windows))]
fn slashes(text: &str) -> String {
    text.to_string()
}

fn kind(entry: &FileEntry) -> char {
    if entry.is_dir {
        'd'
    } else {
        '-'
    }
}

/// The columns of `se ls` plus the time: kind, link flag, size, mtime in
/// milliseconds, path relative to the listed folder.
fn print_line(root: Option<&str>, entry: &FileEntry) {
    let symlink = if entry.is_symlink { "l" } else { "-" };
    println!(
        "{}{symlink}\t{}\t{}\t{}",
        kind(entry),
        entry.size,
        entry.mtime_ms,
        relative(root, entry)
    );
}

fn row(root: Option<&str>, entry: &FileEntry) -> serde_json::Value {
    serde_json::json!({
        "path": relative(root, entry),
        "is_dir": entry.is_dir,
        "is_symlink": entry.is_symlink,
        "size": entry.size,
        "mtime_ms": entry.mtime_ms,
        "hidden": entry.hidden,
    })
}

#[cfg(test)]
mod abgleich_task_tests {
    use super::relative;
    use crate::types::FileEntry;
    use std::sync::Arc;

    fn entry(path: &str, depth: u32) -> FileEntry {
        FileEntry {
            path: Arc::from(path),
            parent: Arc::from(""),
            name: Arc::from(""),
            ext: Arc::from(""),
            size: 0,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir: false,
            is_symlink: false,
            hidden: false,
            system: false,
            depth,
            id: None,
        }
    }

    #[test]
    fn abgleich_task_relative_paths_use_slashes_below_the_scan_root() {
        assert_eq!(
            relative(Some("/data/r"), &entry("/data/r/sub/file.txt", 2)),
            "sub/file.txt"
        );
        assert_eq!(relative(Some("C:/data"), &entry("C:/data/a", 1)), "a");
        assert_eq!(relative(Some("C:/"), &entry("C:/a/b", 2)), "a/b");
        assert_eq!(relative(Some("/"), &entry("/a", 1)), "a");
    }

    #[test]
    fn abgleich_task_an_entry_outside_the_root_keeps_its_full_path() {
        assert_eq!(
            relative(Some("/data/r"), &entry("/data/rx/y", 2)),
            "/data/rx/y",
            "a sibling whose name starts like the root is not below it"
        );
        assert_eq!(relative(None, &entry("/a/b", 2)), "/a/b");
    }

    #[test]
    fn abgleich_task_backslash_is_a_separator_only_on_windows() {
        let name = relative(Some("/r"), &entry("/r/a\\b", 1));
        if cfg!(windows) {
            assert_eq!(name, "a/b");
        } else {
            assert_eq!(name, "a\\b");
        }
    }

    /// A local scan given the verbatim form of a folder reports the same
    /// relative paths as the plain form, with `/` between the components.
    #[cfg(windows)]
    #[test]
    fn abgleich_task_local_scan_in_verbatim_form_gives_slash_relative_paths() {
        use crate::scanner::{ScanMessage, ScanOpts};
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("se_ls_rel_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("sub").join("file.txt"), b"x").unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        let _handle = crate::scanner::start_scan(
            std::path::PathBuf::from(format!(r"\\?\{}", base.display())),
            ScanOpts::everything(None),
            tx,
        );
        let mut root: Option<String> = None;
        let mut below = Vec::new();
        for message in rx {
            match message {
                ScanMessage::Entries(entries) => {
                    for entry in entries {
                        if entry.depth == 0 {
                            root = Some(entry.path.to_string());
                        } else {
                            below.push(relative(root.as_deref(), &entry));
                        }
                    }
                }
                ScanMessage::Done(_) => break,
                _ => {}
            }
        }
        let _ = std::fs::remove_dir_all(&base);
        below.sort();
        assert_eq!(below, vec!["sub".to_string(), "sub/file.txt".to_string()]);
        assert!(!root.unwrap_or_default().starts_with(r"\\?\"));
    }
}
