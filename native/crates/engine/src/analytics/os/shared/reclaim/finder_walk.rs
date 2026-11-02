//! Parallel candidate walk of the duplicate search (Android's own search and
//! a host's search for a peer). Every regular file of at least the minimum
//! size becomes a candidate whose path is kept exactly once; the walk skips
//! links, the app trash, pseudo file systems, the sync engine's own entries
//! (`.se-versions`, `.se-sync-replica`), the app's stages and the folders a
//! host never shows (its own data), and bounds the kept path text.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;

use crate::apptrash::ProtectedAreas;

use super::budget::{LimitExceeded, SharedBudget, MAX_RECLAIM_DEPTH};
use super::cleanup::dir_cleanup_reason;
use super::finder::{FinderLimits, Guard, Issues};
use super::types::ReclaimProgress;
use super::util::to_fwd;

/// Counters reach the shared progress every this many files.
const FLUSH_FILES: u64 = 128;

pub(super) struct Candidate {
    pub(super) root_index: usize,
    pub(super) path: Box<Path>,
    pub(super) size: u64,
    pub(super) mtime_ms: i64,
}

pub(super) struct Walked {
    pub(super) candidates: Vec<Candidate>,
    /// Files of at least the minimum size, kept or not.
    pub(super) eligible: u64,
    /// Eligible files left out because the candidate text budget was spent.
    pub(super) dropped: u64,
    pub(super) limit: Option<LimitExceeded>,
}

/// What the walks of one search share: the depth limit, the candidate text
/// budget and the candidates themselves. Nothing but candidates is kept, so
/// the number of walked entries needs no limit of its own.
pub(super) struct Harvest {
    budget: SharedBudget,
    text: AtomicU64,
    eligible: AtomicU64,
    dropped: AtomicU64,
    found: Mutex<Vec<Candidate>>,
}

impl Default for Harvest {
    fn default() -> Self {
        Self {
            budget: SharedBudget::with_limits(u64::MAX, u64::MAX, MAX_RECLAIM_DEPTH),
            text: AtomicU64::new(0),
            eligible: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            found: Mutex::new(Vec::new()),
        }
    }
}

impl Harvest {
    pub(super) fn finish(self) -> Walked {
        Walked {
            candidates: self.found.into_inner().unwrap_or_else(|p| p.into_inner()),
            eligible: self.eligible.into_inner(),
            dropped: self.dropped.into_inner(),
            limit: self.budget.limit(),
        }
    }
}

/// The walk of one root.
pub(super) struct Walk<'a> {
    pub(super) root: &'a Path,
    pub(super) root_index: usize,
    pub(super) handle: &'a crate::local_access::DirectoryHandle,
    pub(super) progress: &'a ReclaimProgress,
    pub(super) limits: FinderLimits,
    pub(super) protected: &'a ProtectedAreas,
    pub(super) guard: Option<Guard<'a>>,
    pub(super) issues: &'a Issues,
    pub(super) parallel: bool,
    /// Folders never entered (a host's own data inside an export).
    pub(super) excluded: &'a [PathBuf],
    pub(super) harvest: &'a Harvest,
}

impl Walk<'_> {
    pub(super) fn run(&self) {
        if !crate::agent_proto::is_pseudo_dir(&self.root.to_string_lossy()) {
            self.visit(self.root, self.handle, 0, false, true);
        }
    }

    fn stopped(&self) -> bool {
        self.progress.cancel.load(Ordering::Relaxed) || self.harvest.budget.stopped()
    }

    fn areas(&self) -> &[ProtectedAreas] {
        std::slice::from_ref(self.protected)
    }

    fn visit(
        &self,
        dir: &Path,
        handle: &crate::local_access::DirectoryHandle,
        depth: u32,
        inside_cleanup: bool,
        is_root: bool,
    ) {
        if self.stopped() {
            return;
        }
        self.progress.dirs.fetch_add(1, Ordering::Relaxed);
        self.progress.stage.enter_directory(dir);
        self.note_area(dir, is_root);
        let skip_detail = inside_cleanup || dir_cleanup_reason(dir, self.root).is_some();
        if let Some(guard) = self.guard {
            if let Err(error) = guard(dir) {
                self.issues.failed(self.areas(), dir, is_root, || {
                    format!("{}: {error}", to_fwd(dir))
                });
                return;
            }
        }
        let entries = match handle.read_directory() {
            Ok(entries) => entries,
            Err(error) => {
                self.issues.failed(self.areas(), dir, is_root, || {
                    format!("{}: {error}", to_fwd(dir))
                });
                return;
            }
        };
        let mut subdirs: Vec<PathBuf> = Vec::new();
        let mut batch: Vec<Candidate> = Vec::new();
        let (mut files, mut bytes, mut reported) = (0u64, 0u64, (0u64, 0u64));
        for entry in entries {
            if self.stopped() {
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    self.issues.failed(self.areas(), dir, false, || {
                        format!("{}: directory entry: {error}", to_fwd(dir))
                    });
                    continue;
                }
            };
            let path = dir.join(&entry.name);
            let name = entry.name.clone();
            let inspected = path.as_os_str().len().saturating_add(name.len());
            if self
                .harvest
                .budget
                .claim(inspected, depth.saturating_add(1))
                .is_err()
            {
                break;
            }
            let text = name.to_string_lossy();
            if entry.is_link_like
                || entry.kind == crate::local_access::EntryKind::Link
                || skipped_name(&text)
            {
                continue;
            }
            if entry.unreachable || name.to_str().is_none() {
                self.issues.failed(self.areas(), &path, false, || {
                    format!("{}: Name nicht darstellbar", to_fwd(&path))
                });
                continue;
            }
            if entry.kind == crate::local_access::EntryKind::Directory {
                if !crate::agent_proto::is_pseudo_dir(&path.to_string_lossy())
                    && !self.excluded.iter().any(|excluded| excluded == &path)
                {
                    subdirs.push(path);
                    if subdirs.len() == 256 {
                        self.visit_children(
                            dir,
                            handle,
                            std::mem::take(&mut subdirs),
                            depth,
                            skip_detail,
                        );
                        self.progress.stage.enter_directory(dir);
                    }
                }
                continue;
            }
            if entry.kind != crate::local_access::EntryKind::File {
                continue;
            }
            files += 1;
            let size = entry.size;
            bytes = bytes.saturating_add(size);
            if !skip_detail && size >= self.limits.min_bytes {
                let mtime_ms = entry.mtime_ms;
                self.offer(&mut batch, path, size, mtime_ms);
            }
            if files - reported.0 >= FLUSH_FILES {
                self.report(&mut reported, files, bytes);
            }
        }
        self.report(&mut reported, files, bytes);
        if !batch.is_empty() {
            let mut found = self.harvest.found.lock().unwrap_or_else(|p| p.into_inner());
            found.append(&mut batch);
        }
        if self.stopped() {
            return;
        }
        self.visit_children(dir, handle, subdirs, depth, skip_detail);
    }

    fn visit_children(
        &self,
        _: &Path,
        handle: &crate::local_access::DirectoryHandle,
        subdirs: Vec<PathBuf>,
        depth: u32,
        skip_detail: bool,
    ) {
        let next = depth.saturating_add(1);
        let visit = |sub: &PathBuf| match handle.open_child(sub.file_name().unwrap_or_default()) {
            Ok(child) => self.visit(sub, &child, next, skip_detail, false),
            Err(error) => self.issues.failed(self.areas(), sub, false, || {
                format!("{}: {error}", to_fwd(sub))
            }),
        };
        if self.parallel && subdirs.len() > 1 {
            subdirs.par_iter().for_each(visit);
        } else {
            for sub in &subdirs {
                visit(sub);
            }
        }
    }

    /// Records a protected area the walk enters; the root may lie inside one.
    fn note_area(&self, dir: &Path, is_root: bool) {
        if self.protected.is_empty() {
            return;
        }
        let area = if is_root {
            self.protected.area_of(dir)
        } else {
            self.protected.is_area(dir).then_some(dir)
        };
        if let Some(area) = area {
            self.issues.visit(area);
        }
    }

    fn offer(&self, batch: &mut Vec<Candidate>, path: PathBuf, size: u64, mtime_ms: i64) {
        let harvest = self.harvest;
        harvest.eligible.fetch_add(1, Ordering::Relaxed);
        let text = path.as_os_str().len() as u64 + 4 * std::mem::size_of::<Candidate>() as u64;
        let kept = harvest
            .text
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(text)
                    .filter(|next| *next <= self.limits.candidate_text_bytes)
            })
            .is_ok();
        if kept {
            batch.push(Candidate {
                root_index: self.root_index,
                path: path.into_boxed_path(),
                size,
                mtime_ms,
            });
        } else {
            harvest.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn report(&self, reported: &mut (u64, u64), files: u64, bytes: u64) {
        self.progress
            .files
            .fetch_add(files - reported.0, Ordering::Relaxed);
        self.progress
            .bytes
            .fetch_add(bytes - reported.1, Ordering::Relaxed);
        *reported = (files, bytes);
    }
}

/// Entries that are no user files: the app trash, the sync engine's own
/// entries and the app's stages (a crash may leave one behind).
fn skipped_name(name: &str) -> bool {
    crate::apptrash::excluded_name(name)
        || crate::bisync::is_engine_name(name)
        || crate::vfs::is_staging_name(name)
}
