//! Backend-driven directory walk for REMOTE roots (SFTP/FTP/authenticated
//! shares). Streams the same `ScanMessage`s as the local scanner over the same
//! channel, so `app.rs`'s drain loop and the whole UI are unchanged — but it
//! goes through `vfs::Backend::list_dir` instead of `std::fs`. The hot local
//! walk in `scanner.rs` is left completely untouched (isolation): local roots
//! still take the fast path; only remote roots come here.
//!
//! One-level browsing stays serial; recursive scans can list a breadth level in
//! parallel when the backend advertises safe width (Drive/WebDAV).
#![allow(dead_code)] // staged: wired into navigation by the connect-UI step.

#[path = "parallel.rs"]
mod parallel;

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

/// Run a SERVER-SIDE recursive search (the SSH agent's `Search`) under `root`,
/// streaming each match into the scan channel as a flat `FileEntry` whose name
/// is the path RELATIVE to `root` (so the user sees where each hit lives). The
/// app's drain loop / view are unchanged. Falls back to nothing if the backend
/// reports it didn't run the search (the caller decides what to do then).
pub fn start_search_backend(
    backend: BackendHandle,
    root: String,
    spec: crate::agent_proto::SearchSpec,
    tx: Sender<ScanMessage>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let ct = cancel.clone();
    std::thread::Builder::new()
        .name("rscan-search".into())
        .spawn(move || run_search(backend, root, spec, tx, ct))
        .expect("spawn rscan-search thread");
    ScanHandle { cancel }
}

fn run_search(
    backend: BackendHandle,
    root: String,
    spec: crate::agent_proto::SearchSpec,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) {
    use crossbeam_channel::unbounded;
    let start = Instant::now();
    let (htx, hrx) = unbounded::<crate::vfs::SearchHit>();
    // Drive the (blocking) backend search on a worker; batch hits as they arrive.
    let be = backend.clone();
    let root_w = root.clone();
    let cancel_w = cancel.clone();
    let worker = std::thread::Builder::new()
        .name("agent-search".into())
        .spawn(move || {
            be.search(&root_w, &spec, htx, &cancel_w);
        })
        .expect("spawn agent-search thread");

    let root_arc: Arc<str> = Arc::from(root.as_str());
    let mut batch: Vec<FileEntry> = Vec::with_capacity(BATCH);
    let mut scanned: u64 = 0;
    let mut bytes: u64 = 0;
    let mut last_progress = Instant::now();
    while let Ok(hit) = hrx.recv() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let path = join(&root, &hit.rel);
        let base = hit.rel.rsplit('/').next().unwrap_or(&hit.rel);
        let fe = FileEntry {
            path: Arc::from(path.as_str()),
            parent: root_arc.clone(),
            name: Arc::from(hit.rel.as_str()), // show the relative path
            ext: Arc::from(ext_of(base, hit.is_dir).as_str()),
            size: hit.size,
            mtime_ms: hit.mtime_ms,
            btime_ms: 0,
            is_dir: hit.is_dir,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 1,
            id: None,
        };
        scanned += 1;
        if !hit.is_dir {
            bytes += hit.size;
        }
        batch.push(fe);
        if batch.len() >= BATCH {
            let _ = tx.send(ScanMessage::Entries(std::mem::take(&mut batch)));
        }
        if last_progress.elapsed().as_millis() > PROGRESS_MS {
            let _ = tx.send(ScanMessage::Progress(ScanProgress {
                scanned,
                bytes,
                errors: 0,
                elapsed_ms: start.elapsed().as_millis() as u64,
                current_path: root.clone(),
            }));
            last_progress = Instant::now();
        }
    }
    let _ = worker.join();
    if !batch.is_empty() {
        let _ = tx.send(ScanMessage::Entries(std::mem::take(&mut batch)));
    }
    let _ = tx.send(ScanMessage::Done(ScanProgress {
        scanned,
        bytes,
        errors: 0,
        elapsed_ms: start.elapsed().as_millis() as u64,
        current_path: String::new(),
    }));
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
                id: None,
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
            }));
            return;
        }
    }

    if max_depth.is_none() && backend.parallelism() > 1 {
        parallel::run(backend, root, tx, cancel, start);
        return;
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
            let recurse = m.is_dir && !m.is_symlink && max_depth.is_none_or(|max| depth < max);
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
                id: m.id.as_deref().map(Arc::from),
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

    #[test]
    fn recursive_scan_uses_parallel_backend_width() {
        use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
        use std::io::{self, Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct ParallelBackend {
            active: AtomicUsize,
            max_active: AtomicUsize,
        }
        impl ParallelBackend {
            fn enter(&self) {
                let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(40));
                self.active.fetch_sub(1, Ordering::SeqCst);
            }
        }
        impl Backend for ParallelBackend {
            fn scheme(&self) -> Scheme {
                Scheme::GDrive
            }
            fn root_display(&self) -> String {
                "/root".into()
            }
            fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
                self.enter();
                if path == "/root" {
                    return Ok((0..4)
                        .map(|i| VfsMeta {
                            name: format!("d{i}"),
                            is_dir: true,
                            ..Default::default()
                        })
                        .collect());
                }
                let name = path.rsplit('/').next().unwrap_or("x");
                Ok(vec![VfsMeta {
                    name: format!("{name}.txt"),
                    size: 1,
                    ..Default::default()
                }])
            }
            fn stat(&self, _p: &str) -> VfsResult<VfsMeta> {
                Ok(VfsMeta {
                    name: "root".into(),
                    is_dir: true,
                    ..Default::default()
                })
            }
            fn open_read(&self, _p: &str) -> VfsResult<Box<dyn Read + Send>> {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            }
            fn open_write(&self, _p: &str) -> VfsResult<Box<dyn Write + Send>> {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            }
            fn rename(&self, _s: &str, _d: &str) -> VfsResult<()> {
                Ok(())
            }
            fn remove_file(&self, _p: &str) -> VfsResult<()> {
                Ok(())
            }
            fn remove_dir(&self, _p: &str) -> VfsResult<()> {
                Ok(())
            }
            fn mkdir_all(&self, _p: &str) -> VfsResult<()> {
                Ok(())
            }
            fn parallelism(&self) -> usize {
                4
            }
        }

        let typed = Arc::new(ParallelBackend {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
        });
        let be: BackendHandle = typed.clone();
        let (tx, rx) = unbounded();
        start_scan_backend(be, "/root".into(), None, tx);
        let (names, scanned) = drain(&rx);
        assert_eq!(scanned, 8);
        assert!(names.contains("d0.txt") && names.contains("d3.txt"));
        assert!(
            typed.max_active.load(Ordering::SeqCst) > 1,
            "recursive scan did not list sibling folders concurrently"
        );
    }
}
