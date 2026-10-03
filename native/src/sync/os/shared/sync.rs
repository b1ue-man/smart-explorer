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
//! Transfers use exclusive stages, byte-bound signatures and durable version
//! backups before replacements; the pair lock is shared with bisync.
//!
//! The copy pass lists and copies in parallel, as fast as the flows of both
//! connections allow (`sync_pass`); each destination folder is listed once
//! and compared, not probed per file. The delete pass stays serial and runs
//! only after an error-free copy pass.
// The result/progress structs expose more than the current minimal "mirror to a
// folder" UI consumes (per-file `current`, `errors` list, `elapsed_ms`); they're
// the engine's stable API for a richer sync UI later.
#![allow(dead_code)]

use super::sync_pass::{copy_pass_scoped, Report};
use super::sync_scan::Target;
use crate::bisync::SyncOmissions;
use crate::vfs::{Backend, BackendHandle};
use crossbeam_channel::Sender;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const MAX_REPORTED_ERRORS: usize = 100;
const MAX_WALK_DEPTH: usize = 512;

pub(super) struct WalkBudget {
    limits: crate::bisync::SyncLimits,
    nodes: u64,
    text_bytes: u64,
}

impl Default for WalkBudget {
    fn default() -> Self {
        Self {
            limits: crate::bisync::SyncLimits::for_memory(crate::transfer::physical_memory()),
            nodes: 0,
            text_bytes: 0,
        }
    }
}

impl WalkBudget {
    pub(super) fn record(&mut self, path: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_WALK_DEPTH {
            return Err(format!("sync tree exceeds {MAX_WALK_DEPTH} levels"));
        }
        self.nodes = self.nodes.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(path.len() as u64);
        if self.nodes > self.limits.walk_entries {
            return Err(format!(
                "sync tree exceeds {} entries",
                self.limits.walk_entries
            ));
        }
        if self.text_bytes > self.limits.walk_text_bytes {
            return Err(format!(
                "sync relative-path data exceeds {} bytes",
                self.limits.walk_text_bytes
            ));
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
    pub omissions: SyncOmissions,
    pub elapsed_ms: u64,
}

pub enum SyncMsg {
    Progress(SyncProgress),
    Done(SyncResult),
}

pub struct SyncHandle {
    pub cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl SyncHandle {
    /// Transfer ownership for a caller's bounded completion wait. Dropping
    /// an unclaimed handle keeps the existing detached-worker behavior.
    pub fn take_worker(&mut self) -> Option<std::thread::JoinHandle<()>> {
        self.worker.take()
    }
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
    let worker = match std::thread::Builder::new()
        .name("sync-driver".into())
        .spawn(move || run(src, src_root, dst, dst_root, opts, tx, c))
    {
        Ok(worker) => Some(worker),
        Err(error) => {
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
                omissions: SyncOmissions::default(),
            }));
            None
        }
    };
    SyncHandle { cancel, worker }
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
    let omissions = SyncOmissions::new(
        !src.case_sensitive_paths(&src_root) || !dst.case_sensitive_paths(&dst_root),
    );

    if let Err(error) = crate::vfs::validate_sync_roots(&*src, &src_root, &*dst, &dst_root) {
        record_error(&mut stats, &mut errors, "Sync-Pfade", error.to_string());
        let _ = tx.send(SyncMsg::Done(SyncResult {
            stats,
            errors,
            omissions,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }));
        return;
    }

    let run = match super::sync_run::MirrorRun::begin(&*src, &src_root, &*dst, &dst_root) {
        Ok(run) => run,
        Err(error) => {
            record_error(&mut stats, &mut errors, "Sync-Sperre", error.to_string());
            let _ = tx.send(SyncMsg::Done(SyncResult {
                stats,
                errors,
                omissions,
                elapsed_ms: start.elapsed().as_millis() as u64,
            }));
            return;
        }
    };

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
            omissions,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }));
        return;
    }

    // A dry run leaves a missing destination root missing: everything below
    // it counts as "would copy" without any destination listing.
    let root_target = if !opts.dry_run || dst.try_exists(&dst_root).unwrap_or(true) {
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
                omissions,
                elapsed_ms: start.elapsed().as_millis() as u64,
            }));
            return;
        }
        Target::Listed
    } else {
        Target::Absent
    };

    // ── copy/update pass: parallel listing and copying (`sync_pass`) ──
    let Report {
        mut stats,
        mut errors,
        mut omissions,
    } = copy_pass_scoped(
        &*src,
        &src_root,
        &*dst,
        &dst_root,
        opts.dry_run,
        root_target,
        &cancel,
        Report {
            stats,
            errors,
            omissions,
        },
        &tx,
        start,
        &run.versions,
    );

    // ── delete pass (mirror): remove dst entries with no src counterpart ──
    if opts.delete_extra && !cancel.load(Ordering::Relaxed) && stats.errors > 0 {
        record_error(
            &mut stats,
            &mut errors,
            dst_root.clone(),
            "mirror deletion skipped because the copy/source pass reported errors",
        );
    } else if opts.delete_extra && !cancel.load(Ordering::Relaxed) {
        super::sync_delete::delete_extras_scoped(
            &*src,
            &src_root,
            &*dst,
            &dst_root,
            opts.dry_run,
            &cancel,
            &mut stats,
            &mut errors,
            &mut omissions,
            &run.versions,
        );
    }

    if let Err(error) = run.finish(&*dst, &dst_root, &cancel) {
        record_error(
            &mut stats,
            &mut errors,
            &dst_root,
            format!("finish mirror versions: {error}"),
        );
    }

    let _ = tx.send(SyncMsg::Done(SyncResult {
        stats,
        errors,
        omissions,
        elapsed_ms: start.elapsed().as_millis() as u64,
    }));
}

pub(super) fn require_plain_directory(
    backend: &dyn Backend,
    path: &str,
    create: bool,
) -> io::Result<()> {
    if crate::bisync::snapshot_policy::own_path(backend, path) {
        return Err(crate::bisync::apply_boundary::protected(
            crate::bisync::OmissionKind::OwnFile,
        ));
    }
    let metadata = match backend.stat(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
            backend.mkdir_all(path)?;
            backend.stat(path)?
        }
        Err(error) => return Err(error),
    };
    if metadata.is_symlink || !metadata.is_dir || metadata.special {
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
