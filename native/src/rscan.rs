//! Backend-driven directory walk for REMOTE roots (SFTP/FTP/authenticated
//! shares). Streams the same `ScanMessage`s as the local scanner over the same
//! channel, so `app.rs`'s drain loop and the whole UI are unchanged — but it
//! goes through `vfs::Backend::list_dir` instead of `std::fs`. The hot local
//! walk in `scanner.rs` is left completely untouched (isolation): local roots
//! still take the fast path; only remote roots come here.
//!
//! Sequential by design: remote backends report `parallelism() == 1` (one SSH
//! session / one FTP control connection), so a single work queue is correct and
//! avoids hammering the server.
#![allow(dead_code)] // staged: wired into navigation by the connect-UI step.

use crate::scanner::{ScanHandle, ScanMessage};
use crate::types::{FileEntry, ScanProgress};
use crate::vfs::BackendHandle;
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const BATCH: usize = 256;
const PROGRESS_MS: u128 = 150;
const MAX_ERRORS_TRACKED: usize = 500;

fn ext_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() && i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Walk `root` through `backend`, streaming results over `tx`. `max_depth`
/// `Some(1)` = current dir only (flat), `None` = unlimited.
pub fn start_scan_backend(
    backend: BackendHandle,
    root: String,
    max_depth: Option<u32>,
    tx: Sender<ScanMessage>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = cancel.clone();
    std::thread::Builder::new()
        .name("rscan-driver".into())
        .spawn(move || run(backend, root, max_depth, tx, cancel_thread))
        .expect("spawn rscan thread");
    ScanHandle { cancel }
}

fn run(
    backend: BackendHandle,
    root: String,
    max_depth: Option<u32>,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) {
    let start = Instant::now();
    let mut scanned: u64 = 0;
    let mut bytes: u64 = 0;
    let mut errors: u64 = 0;
    let mut failed: Vec<(String, String)> = Vec::new();

    // Root entry — always emitted (the view filter hides what the user doesn't
    // want), mirroring the local scanner.
    match backend.stat(&root) {
        Ok(m) => {
            let parent = match root.rsplit_once('/') {
                Some((p, _)) if !p.is_empty() => p.to_string(),
                _ => String::new(),
            };
            let name = if m.name.is_empty() {
                root.clone()
            } else {
                m.name.clone()
            };
            let entry = FileEntry {
                path: Arc::from(root.as_str()),
                parent: Arc::from(parent.as_str()),
                name: Arc::from(name.as_str()),
                ext: Arc::from(""),
                size: 0,
                mtime_ms: m.mtime_ms,
                btime_ms: m.btime_ms,
                is_dir: true,
                is_symlink: m.is_symlink,
                hidden: m.hidden,
                system: m.system,
                depth: 0,
            };
            let _ = tx.send(ScanMessage::Entries(vec![entry]));
        }
        Err(e) => {
            let _ = tx.send(ScanMessage::Error(format!(
                "Wurzel kann nicht gelesen werden: {} ({})",
                root, e
            )));
            let _ = tx.send(ScanMessage::Done(ScanProgress {
                scanned: 0,
                bytes: 0,
                errors: 1,
                elapsed_ms: start.elapsed().as_millis() as u64,
                current_path: String::new(),
                done: true,
            }));
            return;
        }
    }

    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((root.clone(), 1));
    let mut batch: Vec<FileEntry> = Vec::with_capacity(BATCH);
    let mut last_progress = Instant::now();

    while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let entries = match backend.list_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                errors += 1;
                if failed.len() < MAX_ERRORS_TRACKED {
                    failed.push((dir.clone(), format!("list_dir: {e}")));
                }
                continue;
            }
        };
        let parent_arc: Arc<str> = Arc::from(dir.as_str());
        for m in entries {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let path = join(&dir, &m.name);
            let recurse = m.is_dir && !m.is_symlink && max_depth.map_or(true, |max| depth < max);
            let fe = FileEntry {
                path: Arc::from(path.as_str()),
                parent: parent_arc.clone(),
                name: Arc::from(m.name.as_str()),
                ext: Arc::from(ext_of(&m.name, m.is_dir).as_str()),
                size: m.size,
                mtime_ms: m.mtime_ms,
                btime_ms: m.btime_ms,
                is_dir: m.is_dir,
                is_symlink: m.is_symlink,
                hidden: m.hidden,
                system: m.system,
                depth,
            };
            scanned += 1;
            if !m.is_dir {
                bytes += m.size;
            }
            if recurse {
                queue.push_back((path, depth + 1));
            }
            batch.push(fe);
            if batch.len() >= BATCH {
                let _ = tx.send(ScanMessage::Entries(std::mem::take(&mut batch)));
            }
        }

        if !batch.is_empty() {
            let _ = tx.send(ScanMessage::Entries(std::mem::take(&mut batch)));
        }
        if last_progress.elapsed().as_millis() > PROGRESS_MS {
            let _ = tx.send(ScanMessage::Progress(ScanProgress {
                scanned,
                bytes,
                errors,
                elapsed_ms: start.elapsed().as_millis() as u64,
                current_path: dir.clone(),
                done: false,
            }));
            last_progress = Instant::now();
        }
    }

    if !failed.is_empty() {
        let _ = tx.send(ScanMessage::FailedPaths(failed));
    }
    let _ = tx.send(ScanMessage::Done(ScanProgress {
        scanned,
        bytes,
        errors,
        elapsed_ms: start.elapsed().as_millis() as u64,
        current_path: String::new(),
        done: true,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::LocalBackend;
    use crossbeam_channel::unbounded;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn temp_tree() -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("rscan_{}_{}", std::process::id(), nanos));
        std::fs::create_dir_all(p.join("sub")).unwrap();
        std::fs::write(p.join("a.txt"), b"hello").unwrap();
        std::fs::write(p.join("sub").join("b.dat"), b"xy").unwrap();
        p
    }

    fn drain(rx: &crossbeam_channel::Receiver<ScanMessage>) -> (HashSet<String>, u64) {
        let mut names = HashSet::new();
        let mut scanned = 0;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(ScanMessage::Entries(v)) => {
                    for e in v {
                        names.insert(e.name.to_string());
                    }
                }
                Ok(ScanMessage::Done(p)) => {
                    scanned = p.scanned;
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        (names, scanned)
    }

    #[test]
    fn walks_full_tree_via_backend() {
        let dir = temp_tree();
        let root = dir.to_string_lossy().replace('\\', "/");
        let be: BackendHandle = Arc::new(LocalBackend::new(&root));
        let (tx, rx) = unbounded();
        start_scan_backend(be, root, None, tx);
        let (names, scanned) = drain(&rx);
        assert!(names.contains("a.txt"), "names: {names:?}");
        assert!(names.contains("sub"));
        assert!(names.contains("b.dat"), "should recurse into sub");
        assert_eq!(scanned, 3); // a.txt, sub, sub/b.dat
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flat_depth_one_does_not_recurse() {
        let dir = temp_tree();
        let root = dir.to_string_lossy().replace('\\', "/");
        let be: BackendHandle = Arc::new(LocalBackend::new(&root));
        let (tx, rx) = unbounded();
        start_scan_backend(be, root, Some(1), tx);
        let (names, scanned) = drain(&rx);
        assert!(names.contains("a.txt") && names.contains("sub"));
        assert!(!names.contains("b.dat"), "depth 1 must not recurse");
        assert_eq!(scanned, 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}
