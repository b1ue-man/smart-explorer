//! One-way mirror between any two `vfs::Backend`s (local↔remote, remote↔local,
//! remote↔remote). Because it speaks only the `Backend` interface, the same
//! engine backs every pairing — local→SFTP, WebDAV→local, etc.
//!
//! Semantics (one-way, src → dst):
//!  * Copy a file when it's missing in dst, or its size differs, or src is
//!    newer (mtime). Otherwise skip.
//!  * `delete_extra` additionally removes dst files/dirs that don't exist in src
//!    (mirror mode). Off by default — the safe one-way is copy/update only.
//!  * `dry_run` reports what would change without writing.
//!
//! Streaming copy goes through `open_read`/`open_write` + an explicit `flush`
//! so remote writers (FTP/WebDAV buffer-then-PUT) surface upload errors.
// The result/progress structs expose more than the current minimal "mirror to a
// folder" UI consumes (per-file `current`, `errors` list, `elapsed_ms`); they're
// the engine's stable API for a richer sync UI later.
#![allow(dead_code)]

use crate::vfs::{Backend, BackendHandle};
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Default, Clone, Debug)]
pub struct SyncStats {
    pub copied: u64,
    pub skipped: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub errors: u64,
}

pub struct SyncProgress {
    pub current: String,
    pub stats: SyncStats,
}

pub struct SyncResult {
    pub stats: SyncStats,
    pub errors: Vec<(String, String)>,
    pub elapsed_ms: u64,
}

pub enum SyncMsg {
    Progress(SyncProgress),
    Done(SyncResult),
}

pub struct SyncHandle {
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
pub struct SyncOptions {
    pub delete_extra: bool,
    pub dry_run: bool,
}

fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else {
        format!("{}/{}", root.trim_end_matches('/'), rel)
    }
}

fn rel_of(path: &str, root: &str) -> String {
    let r = root.trim_end_matches('/');
    if let Some(rest) = path.strip_prefix(r) {
        rest.trim_start_matches('/').to_string()
    } else {
        path.trim_start_matches('/').to_string()
    }
}

fn parent_of(path: &str) -> Option<String> {
    let t = path.trim_end_matches('/');
    t.rfind('/').map(|i| {
        if i == 0 {
            "/".to_string()
        } else {
            t[..i].to_string()
        }
    })
}

fn copy_stream(src: &dyn Backend, sp: &str, dst: &dyn Backend, dp: &str) -> io::Result<u64> {
    if let Some(parent) = parent_of(dp) {
        let _ = dst.mkdir_all(&parent);
    }
    let mut r = src.open_read(sp)?;
    let mut w = dst.open_write(dp)?;
    let n = io::copy(&mut r, &mut w)?;
    w.flush()?; // force remote commit so upload errors surface here
    Ok(n)
}

pub fn start_sync(
    src: BackendHandle,
    src_root: String,
    dst: BackendHandle,
    dst_root: String,
    opts: SyncOptions,
    tx: Sender<SyncMsg>,
) -> SyncHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    std::thread::Builder::new()
        .name("sync-driver".into())
        .spawn(move || run(src, src_root, dst, dst_root, opts, tx, c))
        .expect("spawn sync thread");
    SyncHandle { cancel }
}

fn run(
    src: BackendHandle,
    src_root: String,
    dst: BackendHandle,
    dst_root: String,
    opts: SyncOptions,
    tx: Sender<SyncMsg>,
    cancel: Arc<AtomicBool>,
) {
    let start = Instant::now();
    let mut stats = SyncStats::default();
    let mut errors: Vec<(String, String)> = Vec::new();
    let mut last_progress = Instant::now();

    if !opts.dry_run {
        let _ = dst.mkdir_all(&dst_root);
    }

    // ── copy/update pass (BFS over src) ──
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(src_root.clone());
    while let Some(dir) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let entries = match src.list_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                stats.errors += 1;
                errors.push((dir, e.to_string()));
                continue;
            }
        };
        for m in entries {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let sp = join(&dir, &m.name);
            let rel = rel_of(&sp, &src_root);
            let dp = join(&dst_root, &rel);
            if m.is_dir {
                if !opts.dry_run {
                    let _ = dst.mkdir_all(&dp);
                }
                queue.push_back(sp);
                continue;
            }
            let need = match dst.stat(&dp) {
                Err(_) => true,
                Ok(dm) => dm.size != m.size || m.mtime_ms > dm.mtime_ms,
            };
            if !need {
                stats.skipped += 1;
                continue;
            }
            if opts.dry_run {
                stats.copied += 1;
            } else {
                match copy_stream(&*src, &sp, &*dst, &dp) {
                    Ok(n) => {
                        stats.copied += 1;
                        stats.bytes += n;
                    }
                    Err(e) => {
                        stats.errors += 1;
                        errors.push((sp.clone(), e.to_string()));
                    }
                }
            }
            if last_progress.elapsed().as_millis() > 150 {
                let _ = tx.send(SyncMsg::Progress(SyncProgress {
                    current: dp.clone(),
                    stats: stats.clone(),
                }));
                last_progress = Instant::now();
            }
        }
    }

    // ── delete pass (mirror): remove dst entries with no src counterpart ──
    if opts.delete_extra && !cancel.load(Ordering::Relaxed) {
        let mut files: Vec<String> = Vec::new();
        let mut dirs: Vec<String> = Vec::new();
        let mut dq: VecDeque<String> = VecDeque::new();
        dq.push_back(dst_root.clone());
        while let Some(dir) = dq.pop_front() {
            let entries = match dst.list_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for m in entries {
                let dp = join(&dir, &m.name);
                let rel = rel_of(&dp, &dst_root);
                let sp = join(&src_root, &rel);
                let in_src = src.stat(&sp).is_ok();
                if m.is_dir {
                    dq.push_back(dp.clone());
                    if !in_src {
                        dirs.push(dp);
                    }
                } else if !in_src {
                    files.push(dp);
                }
            }
        }
        if !opts.dry_run {
            for f in &files {
                if dst.remove_file(f).is_ok() {
                    stats.deleted += 1;
                }
            }
            // deepest dirs first so they're empty when removed
            dirs.sort_by_key(|d| std::cmp::Reverse(d.len()));
            for d in &dirs {
                if dst.remove_dir(d).is_ok() {
                    stats.deleted += 1;
                }
            }
        } else {
            stats.deleted += (files.len() + dirs.len()) as u64;
        }
    }

    let _ = tx.send(SyncMsg::Done(SyncResult {
        stats,
        errors,
        elapsed_ms: start.elapsed().as_millis() as u64,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::LocalBackend;
    use crossbeam_channel::unbounded;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("sync_{}_{}_{}", tag, std::process::id(), nanos));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn fwd(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }
    fn wait(rx: &crossbeam_channel::Receiver<SyncMsg>) -> SyncResult {
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(SyncMsg::Done(r)) => return r,
                Ok(_) => {}
                Err(_) => panic!("sync timed out"),
            }
        }
    }
    fn handles(src: &std::path::Path, dst: &std::path::Path) -> (BackendHandle, BackendHandle) {
        (
            Arc::new(LocalBackend::new(&fwd(src))),
            Arc::new(LocalBackend::new(&fwd(dst))),
        )
    }

    #[test]
    fn mirrors_tree_and_updates_changed() {
        let src = tmp("src");
        let dst = tmp("dst");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"world!!").unwrap();

        let (sb, db) = handles(&src, &dst);
        let (tx, rx) = unbounded();
        start_sync(
            sb,
            fwd(&src),
            db,
            fwd(&dst),
            SyncOptions {
                delete_extra: false,
                dry_run: false,
            },
            tx,
        );
        let r = wait(&rx);
        assert_eq!(r.stats.errors, 0);
        assert_eq!(r.stats.copied, 2);
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dst.join("sub/b.txt")).unwrap(), b"world!!");

        // change a source file (different size) → only it is re-copied
        std::fs::write(src.join("a.txt"), b"hello world").unwrap();
        let (sb, db) = handles(&src, &dst);
        let (tx, rx) = unbounded();
        start_sync(
            sb,
            fwd(&src),
            db,
            fwd(&dst),
            SyncOptions {
                delete_extra: false,
                dry_run: false,
            },
            tx,
        );
        let r = wait(&rx);
        assert_eq!(r.stats.copied, 1);
        assert_eq!(r.stats.skipped, 1);
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"hello world");

        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn delete_extra_removes_orphans() {
        let src = tmp("src2");
        let dst = tmp("dst2");
        std::fs::write(src.join("keep.txt"), b"x").unwrap();
        std::fs::write(dst.join("keep.txt"), b"x").unwrap();
        std::fs::write(dst.join("orphan.txt"), b"y").unwrap();
        std::fs::create_dir_all(dst.join("gone")).unwrap();
        std::fs::write(dst.join("gone/z.txt"), b"z").unwrap();

        let (sb, db) = handles(&src, &dst);
        let (tx, rx) = unbounded();
        start_sync(
            sb,
            fwd(&src),
            db,
            fwd(&dst),
            SyncOptions {
                delete_extra: true,
                dry_run: false,
            },
            tx,
        );
        let r = wait(&rx);
        assert!(dst.join("keep.txt").exists());
        assert!(
            !dst.join("orphan.txt").exists(),
            "orphan file must be deleted"
        );
        assert!(
            !dst.join("gone/z.txt").exists(),
            "orphan dir contents deleted"
        );
        assert!(r.stats.deleted >= 2);

        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn dry_run_writes_nothing() {
        let src = tmp("src3");
        let dst = tmp("dst3");
        std::fs::write(src.join("a.txt"), b"data").unwrap();
        let (sb, db) = handles(&src, &dst);
        let (tx, rx) = unbounded();
        start_sync(
            sb,
            fwd(&src),
            db,
            fwd(&dst),
            SyncOptions {
                delete_extra: false,
                dry_run: true,
            },
            tx,
        );
        let r = wait(&rx);
        assert_eq!(r.stats.copied, 1);
        assert!(!dst.join("a.txt").exists(), "dry-run must not write");
        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }
}
