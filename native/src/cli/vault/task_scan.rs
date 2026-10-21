//! The search for the plaintext marker (fixture `marker`, suite checks
//! `vault_task_klartext_marker_scan` and `…_drive`): the tests put the
//! marker into the names and the contents of a vault and run actions on
//! them; afterwards no file below the environment of the test may carry it
//! in its name or its bytes (ASCII or UTF-16, as `rdt_task/vault.py`
//! `scan_tree` searches), except in the folder of the cleartext inputs.
use std::path::{Path, PathBuf};

/// The marker of the stage (`SE_VAULT_TASK_MARKER`, made by the fixture).
pub(crate) fn marker_token() -> String {
    let token = std::env::var("SE_VAULT_TASK_MARKER")
        .expect("SE_VAULT_TASK_MARKER fehlt: nur in den Stufen mit fixture=marker");
    assert!(token.len() >= 12, "Markierung zu kurz: {token}");
    token
}

/// The mount points of the FUSE file systems of this process.
fn fuse_mounts() -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let (head, tail) = line.split_once(" - ")?;
            if !tail.split(' ').next()?.starts_with("fuse") {
                return None;
            }
            let point = head.split(' ').nth(4)?;
            Some(PathBuf::from(point.replace("\\040", " ")))
        })
        .collect()
}

/// `path` with its links resolved, as `/proc/self/mountinfo` writes mount
/// points; `path` itself when that fails (a mount whose server is gone).
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether a FUSE file system is mounted at `path`.
pub(crate) fn is_fuse_mount(path: &Path) -> bool {
    let path = resolved(path);
    fuse_mounts().iter().any(|mount| *mount == path)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Every place below `root` where `token` stands in a name or in the bytes
/// of a file, outside the folders `skip`. A FUSE mount is not entered; it is
/// a finding itself (the suite does the same).
pub(crate) fn marker_findings(root: &Path, token: &str, skip: &[PathBuf]) -> Vec<String> {
    let utf16: Vec<u8> = token.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let needles = [token.as_bytes().to_vec(), utf16];
    let mounts = fuse_mounts();
    let mut findings = Vec::new();
    walk(root, token, &needles, skip, &mounts, &mut findings);
    findings
}

fn walk(
    dir: &Path,
    token: &str,
    needles: &[Vec<u8>],
    skip: &[PathBuf],
    mounts: &[PathBuf],
    findings: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if skip.iter().any(|skipped| path.starts_with(skipped)) {
            continue;
        }
        if entry.file_name().to_string_lossy().contains(token) {
            findings.push(format!("{}: Markierung im Namen", path.display()));
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if mounts.contains(&resolved(&path)) {
                findings.push(format!(
                    "{}: FUSE-Laufwerk noch eingehängt (nicht durchsucht)",
                    path.display()
                ));
            } else {
                walk(&path, token, needles, skip, mounts, findings);
            }
        } else if kind.is_file() {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if needles.iter().any(|needle| contains(&bytes, needle)) {
                findings.push(format!("{}: Markierung im Inhalt", path.display()));
            }
        }
    }
}
