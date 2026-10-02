//! Parallel candidate walk of the Android duplicate search. Every regular
//! file of at least the minimum size becomes a candidate whose path is kept
//! exactly once; the walk shares the find-and-reclaim walk budget and skip
//! rules (links, app trash, pseudo file systems, details below build/cache
//! folders) and bounds the kept path text separately.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;

use crate::apptrash::ProtectedAreas;

use super::budget::{LimitExceeded, SharedBudget};
use super::cleanup::dir_cleanup_reason;
use super::finder::{FinderLimits, Guard, Issues};
use super::types::ReclaimProgress;
use super::util::{systemtime_ms, to_fwd};

/// Counters reach the shared progress every this many files.
const FLUSH_FILES: u64 = 128;

pub(super) struct Candidate {
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

pub(super) struct Walk<'a> {
    root: &'a Path,
    progress: &'a ReclaimProgress,
    limits: FinderLimits,
    protected: &'a ProtectedAreas,
    guard: Option<Guard<'a>>,
    issues: &'a Issues,
    parallel: bool,
    budget: SharedBudget,
    text: AtomicU64,
    eligible: AtomicU64,
    dropped: AtomicU64,
    found: Mutex<Vec<Candidate>>,
}

impl<'a> Walk<'a> {
    pub(super) fn new(
        root: &'a Path,
        progress: &'a ReclaimProgress,
        limits: FinderLimits,
        protected: &'a ProtectedAreas,
        guard: Option<Guard<'a>>,
        issues: &'a Issues,
        parallel: bool,
    ) -> Self {
        Self {
            root,
            progress,
            limits,
            protected,
            guard,
            issues,
            parallel,
            budget: SharedBudget::default(),
            text: AtomicU64::new(0),
            eligible: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            found: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn run(&self) {
        if !crate::agent_proto::is_pseudo_dir(&self.root.to_string_lossy()) {
            self.visit(self.root, 0, false, true);
        }
    }

    pub(super) fn finish(self) -> Walked {
        Walked {
            candidates: self.found.into_inner().unwrap_or_else(|p| p.into_inner()),
            eligible: self.eligible.into_inner(),
            dropped: self.dropped.into_inner(),
            limit: self.budget.limit(),
        }
    }

    fn stopped(&self) -> bool {
        self.progress.cancel.load(Ordering::Relaxed) || self.budget.stopped()
    }

    fn visit(&self, dir: &Path, depth: u32, inside_cleanup: bool, is_root: bool) {
        if self.stopped() {
            return;
        }
        self.progress.dirs.fetch_add(1, Ordering::Relaxed);
        self.progress.stage.enter_directory(dir);
        self.note_area(dir, is_root);
        let skip_detail = inside_cleanup || dir_cleanup_reason(dir, self.root).is_some();
        if let Some(guard) = self.guard {
            if let Err(error) = guard(dir) {
                self.issues.failed(self.protected, dir, is_root, || {
                    format!("{}: {error}", to_fwd(dir))
                });
                return;
            }
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                self.issues.failed(self.protected, dir, is_root, || {
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
                    self.issues.failed(self.protected, dir, false, || {
                        format!("{}: directory entry: {error}", to_fwd(dir))
                    });
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name();
            let inspected = path.as_os_str().len().saturating_add(name.len());
            if self
                .budget
                .claim(inspected, depth.saturating_add(1))
                .is_err()
            {
                break;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    self.issues.failed(self.protected, &path, false, || {
                        format!("{}: {error}", to_fwd(&path))
                    });
                    continue;
                }
            };
            if file_type.is_symlink() || crate::apptrash::excluded_name(&name.to_string_lossy()) {
                continue;
            }
            if file_type.is_dir() {
                if !crate::agent_proto::is_pseudo_dir(&path.to_string_lossy()) {
                    subdirs.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            files += 1;
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.issues.failed(self.protected, &path, false, || {
                        format!("{}: {error}", to_fwd(&path))
                    });
                    continue;
                }
            };
            let size = metadata.len();
            bytes = bytes.saturating_add(size);
            if !skip_detail && size >= self.limits.min_bytes {
                let mtime_ms = metadata.modified().map(systemtime_ms).unwrap_or(0);
                self.offer(&mut batch, path, size, mtime_ms);
            }
            if files - reported.0 >= FLUSH_FILES {
                self.report(&mut reported, files, bytes);
            }
        }
        self.report(&mut reported, files, bytes);
        if !batch.is_empty() {
            let mut found = self.found.lock().unwrap_or_else(|p| p.into_inner());
            found.append(&mut batch);
        }
        if self.stopped() {
            return;
        }
        let next = depth.saturating_add(1);
        if self.parallel && subdirs.len() > 1 {
            subdirs
                .par_iter()
                .for_each(|sub| self.visit(sub, next, skip_detail, false));
        } else {
            for sub in &subdirs {
                self.visit(sub, next, skip_detail, false);
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
        self.eligible.fetch_add(1, Ordering::Relaxed);
        let text = path.as_os_str().len() as u64;
        let kept = self
            .text
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(text)
                    .filter(|next| *next <= self.limits.candidate_text_bytes)
            })
            .is_ok();
        if kept {
            batch.push(Candidate {
                path: path.into_boxed_path(),
                size,
                mtime_ms,
            });
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
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
