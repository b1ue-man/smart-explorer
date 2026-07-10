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

use super::sync_copy::copy_stream;
use crate::vfs::{Backend, BackendHandle};
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const MAX_REPORTED_ERRORS: usize = 100;
const MAX_WALK_NODES: u64 = 1_000_000;
const MAX_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_WALK_DEPTH: usize = 512;

#[derive(Default)]
pub(super) struct WalkBudget {
    nodes: u64,
    text_bytes: u64,
}

impl WalkBudget {
    pub(super) fn record(&mut self, path: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_WALK_DEPTH {
            return Err(format!("sync tree exceeds {MAX_WALK_DEPTH} levels"));
        }
        self.nodes = self.nodes.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(path.len() as u64);
        if self.nodes > MAX_WALK_NODES {
            return Err(format!("sync tree exceeds {MAX_WALK_NODES} entries"));
        }
        if self.text_bytes > MAX_WALK_TEXT_BYTES {
            return Err("sync path data exceeds 128 MiB".to_string());
        }
        Ok(())
    }
}

#[derive(Default, Clone, Debug)]
pub struct SyncStats {
    pub copied: u64,
    pub skipped: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub errors: u64,
}

#[derive(Clone, Debug)]
pub struct SyncProgress {
    pub current: String,
    pub stats: SyncStats,
    pub elapsed_ms: u64,
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

pub(super) fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else {
        format!("{}/{}", root.trim_end_matches('/'), rel)
    }
}

pub(super) fn rel_of(path: &str, root: &str) -> String {
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

pub(super) fn record_error(
    stats: &mut SyncStats,
    errors: &mut Vec<(String, String)>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    stats.errors = stats.errors.saturating_add(1);
    if errors.len() < MAX_REPORTED_ERRORS {
        errors.push((path.into(), message.into()));
        return;
    }
    let suppressed = stats.errors.saturating_sub(MAX_REPORTED_ERRORS as u64);
    let summary = (
        String::new(),
        format!("{suppressed} weitere Synchronisierungsfehler unterdrückt"),
    );
    if errors.len() == MAX_REPORTED_ERRORS {
        errors.push(summary);
    } else {
        errors[MAX_REPORTED_ERRORS] = summary;
    }
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
    let spawn_errors = tx.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("sync-driver".into())
        .spawn(move || run(src, src_root, dst, dst_root, opts, tx, c))
    {
        let _ = spawn_errors.send(SyncMsg::Done(SyncResult {
            stats: SyncStats {
                errors: 1,
                ..Default::default()
            },
            errors: vec![(
                "sync-driver".into(),
                format!("worker start failed: {error}"),
            )],
            elapsed_ms: 0,
        }));
    }
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

    if let Err(error) = require_plain_directory(&*src, &src_root, false) {
        record_error(
            &mut stats,
            &mut errors,
            src_root.clone(),
            format!("invalid sync source root: {error}"),
        );
        let _ = tx.send(SyncMsg::Done(SyncResult {
            stats,
            errors,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }));
        return;
    }

    if !opts.dry_run || dst.try_exists(&dst_root).unwrap_or(true) {
        if let Err(error) = require_plain_directory(&*dst, &dst_root, !opts.dry_run) {
            record_error(
                &mut stats,
                &mut errors,
                dst_root.clone(),
                format!("invalid sync destination root: {error}"),
            );
            let _ = tx.send(SyncMsg::Done(SyncResult {
                stats,
                errors,
                elapsed_ms: start.elapsed().as_millis() as u64,
            }));
            return;
        }
    }

    // ── copy/update pass (BFS over src) ──
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let mut source_budget = WalkBudget::default();
    if let Err(error) = source_budget.record(&src_root, 0) {
        record_error(&mut stats, &mut errors, src_root.clone(), error);
    } else {
        queue.push_back((src_root.clone(), 0));
    }
    'copy_walk: while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Err(error) = require_plain_directory(&*src, &dir, false) {
            record_error(
                &mut stats,
                &mut errors,
                dir.clone(),
                format!("source directory changed before traversal: {error}"),
            );
            continue;
        }
        let entries = match src.list_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                record_error(&mut stats, &mut errors, dir, e.to_string());
                continue;
            }
        };
        let mut child_names = std::collections::HashSet::new();
        for m in entries {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if let Err(error) = crate::vfs::validate_child_name(&m.name) {
                record_error(&mut stats, &mut errors, dir.clone(), error.to_string());
                continue;
            }
            if !child_names.insert(m.name.clone()) {
                record_error(
                    &mut stats,
                    &mut errors,
                    dir.clone(),
                    format!("backend returned duplicate child name: {:?}", m.name),
                );
                continue;
            }
            if m.is_symlink {
                record_error(
                    &mut stats,
                    &mut errors,
                    dir.clone(),
                    format!("link-like source entry is not synchronized: {:?}", m.name),
                );
                continue;
            }
            let sp = join(&dir, &m.name);
            if let Err(error) = source_budget.record(&sp, depth + 1) {
                record_error(&mut stats, &mut errors, sp, error);
                break 'copy_walk;
            }
            let rel = rel_of(&sp, &src_root);
            let dp = join(&dst_root, &rel);
            if m.is_dir {
                if !opts.dry_run {
                    if let Err(error) = require_plain_directory(&*dst, &dp, true) {
                        record_error(
                            &mut stats,
                            &mut errors,
                            dp,
                            format!("create destination directory: {error}"),
                        );
                        continue;
                    }
                }
                queue.push_back((sp, depth + 1));
                continue;
            }
            let (need, destination_expected) = match dst.stat(&dp) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => (true, None),
                Err(error) => {
                    record_error(
                        &mut stats,
                        &mut errors,
                        dp.clone(),
                        format!("inspect destination: {error}"),
                    );
                    continue;
                }
                Ok(dm) if dm.is_dir || dm.is_symlink => {
                    record_error(
                        &mut stats,
                        &mut errors,
                        dp.clone(),
                        "destination is a directory or link-like entry",
                    );
                    continue;
                }
                Ok(dm) => (dm.size != m.size || m.mtime_ms > dm.mtime_ms, Some(dm)),
            };
            if !need {
                stats.skipped += 1;
                continue;
            }
            if opts.dry_run {
                stats.copied += 1;
            } else {
                match copy_stream(
                    &*src,
                    &sp,
                    &m,
                    &*dst,
                    &dp,
                    destination_expected.as_ref(),
                    &cancel,
                ) {
                    Ok(n) => {
                        stats.copied += 1;
                        stats.bytes += n;
                    }
                    Err(e) => {
                        record_error(&mut stats, &mut errors, sp.clone(), e.to_string());
                    }
                }
            }
            if last_progress.elapsed().as_millis() > 150 {
                let _ = tx.send(SyncMsg::Progress(SyncProgress {
                    current: dp.clone(),
                    stats: stats.clone(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                }));
                last_progress = Instant::now();
            }
        }
    }

    // ── delete pass (mirror): remove dst entries with no src counterpart ──
    if opts.delete_extra && !cancel.load(Ordering::Relaxed) && stats.errors > 0 {
        record_error(
            &mut stats,
            &mut errors,
            dst_root.clone(),
            "mirror deletion skipped because the copy/source pass reported errors",
        );
    } else if opts.delete_extra && !cancel.load(Ordering::Relaxed) {
        super::sync_delete::delete_extras(
            &*src,
            &src_root,
            &*dst,
            &dst_root,
            opts.dry_run,
            &cancel,
            &mut stats,
            &mut errors,
        );
    }

    let _ = tx.send(SyncMsg::Done(SyncResult {
        stats,
        errors,
        elapsed_ms: start.elapsed().as_millis() as u64,
    }));
}

pub(super) fn require_plain_directory(
    backend: &dyn Backend,
    path: &str,
    create: bool,
) -> io::Result<()> {
    let metadata = match backend.stat(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
            backend.mkdir_all(path)?;
            backend.stat(path)?
        }
        Err(error) => return Err(error),
    };
    if metadata.is_symlink || !metadata.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("directory root is link-like or not a directory: {path}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "sync_queue_tests.rs"]
mod queue_tests;
#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
