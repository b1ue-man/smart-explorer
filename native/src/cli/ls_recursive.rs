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
    let root = target.path.trim_end_matches(['/', '\\']).to_string();
    let mut rows = Vec::new();
    let mut failed = Vec::new();
    let mut error = None;
    for message in rx {
        match message {
            ScanMessage::Entries(entries) => {
                for entry in entries.into_iter().filter(|entry| entry.depth > 0) {
                    if json {
                        rows.push(row(&root, &entry));
                    } else {
                        print_line(&root, &entry);
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

fn relative<'a>(root: &str, entry: &'a FileEntry) -> &'a str {
    let path: &str = &entry.path;
    path.strip_prefix(root)
        .map(|rest| rest.trim_start_matches(['/', '\\']))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
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
fn print_line(root: &str, entry: &FileEntry) {
    let symlink = if entry.is_symlink { "l" } else { "-" };
    println!(
        "{}{symlink}\t{}\t{}\t{}",
        kind(entry),
        entry.size,
        entry.mtime_ms,
        relative(root, entry)
    );
}

fn row(root: &str, entry: &FileEntry) -> serde_json::Value {
    serde_json::json!({
        "path": relative(root, entry),
        "is_dir": entry.is_dir,
        "is_symlink": entry.is_symlink,
        "size": entry.size,
        "mtime_ms": entry.mtime_ms,
        "hidden": entry.hidden,
    })
}
