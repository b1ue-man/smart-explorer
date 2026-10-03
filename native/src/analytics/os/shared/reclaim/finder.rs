//! Duplicate search of the Android app. Every file of at least the minimum
//! size is a candidate (bounded only by the walk budget and a candidate text
//! budget, both reported), only sizes shared by two or more files are read,
//! and the first/last-bytes and full-content comparisons run in parallel with
//! hardware-accelerated SHA-256 (`ring`). The desktop's find-and-reclaim
//! keeps its walk with the 200 largest candidates (`local.rs`).
use std::io;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::analytics::{
    protected_count, protected_text, thousands, ProtectedOmission, ProtectedTally,
};
use crate::apptrash::ProtectedAreas;

use super::budget::describe_scan_limit;
use super::finder_compare::{compare_candidates, Compare};
use super::finder_walk::{Harvest, Walk};
use super::types::{DuplicateGroup, ReclaimProgress, ReclaimReport};
use super::util::{push_bounded_error, to_fwd};

/// Path text kept for candidates at least (one path per candidate file).
pub(super) const MAX_CANDIDATE_TEXT_BYTES: u64 = 64 * 1024 * 1024;
/// Path text kept for candidates at most: 1 GiB holds about ten million paths.
const CANDIDATE_TEXT_CEILING: u64 = 1024 * 1024 * 1024;

/// Candidate path text the device can hold: 1/64 of its memory (paths are the
/// only data a search keeps per file), at least the former fixed budget.
pub(crate) fn candidate_text_budget() -> u64 {
    crate::transfer::physical_memory()
        .map_or(MAX_CANDIDATE_TEXT_BYTES, |memory| memory / 64)
        .clamp(MAX_CANDIDATE_TEXT_BYTES, CANDIDATE_TEXT_CEILING)
}
/// Bytes compared at each end of a same-size candidate, as on the desktop.
pub(super) const SAMPLE_BYTES: u64 = 64 * 1024;

pub(crate) type Guard<'a> = &'a (dyn Fn(&Path) -> io::Result<()> + Sync);

/// One root of a search and the protected areas its walk may meet.
pub(crate) struct FinderRoot {
    pub(crate) path: std::path::PathBuf,
    pub(crate) protected: ProtectedAreas,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FinderLimits {
    pub(crate) min_bytes: u64,
    pub(crate) candidate_text_bytes: u64,
    pub(crate) threads: usize,
}

/// Everything a duplicate search reports besides its groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DuplicateSummary {
    pub files: u64,
    pub bytes: u64,
    /// Files of at least the minimum size.
    pub candidates: u64,
    /// Candidates whose contents were compared (their size is shared).
    pub compared: u64,
    pub groups: u64,
    pub protected: Vec<ProtectedOmission>,
    pub errors: Vec<String>,
    pub suppressed_errors: u64,
    /// Why the search saw or compared less than everything (German).
    pub limits: Vec<String>,
}

/// `reclaim.summary` of the Android facade.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateSummaryView {
    pub files: u64,
    pub bytes: u64,
    pub candidates: u64,
    pub compared: u64,
    pub groups: u64,
    pub protected_count: u64,
    pub protected_text: String,
    pub error_count: u64,
    pub error_text: String,
    pub limit: Option<String>,
}

impl DuplicateSummary {
    pub fn error_count(&self) -> u64 {
        (self.errors.len() as u64).saturating_add(self.suppressed_errors)
    }

    pub fn view(&self) -> DuplicateSummaryView {
        let mut errors = self.errors.clone();
        if self.suppressed_errors > 0 {
            errors.push(format!("… {} weitere", thousands(self.suppressed_errors)));
        }
        DuplicateSummaryView {
            files: self.files,
            bytes: self.bytes,
            candidates: self.candidates,
            compared: self.compared,
            groups: self.groups,
            protected_count: protected_count(&self.protected),
            protected_text: protected_text(&self.protected),
            error_count: self.error_count(),
            error_text: errors.join("\n"),
            limit: (!self.limits.is_empty()).then(|| self.limits.join("\n")),
        }
    }
}

#[derive(Debug)]
pub struct DuplicateReport {
    /// All groups, largest reclaimable space first.
    pub groups: Vec<DuplicateGroup>,
    pub summary: DuplicateSummary,
    /// The root itself could not be read (outside protected areas).
    pub root_error: Option<String>,
}

impl DuplicateReport {
    /// A remote search (`scan_reclaim_backend`, provider or agent MD5) in the
    /// same shape; its candidate and group caps stay and are named.
    pub fn from_reclaim(report: ReclaimReport) -> Self {
        let mut limits = Vec::new();
        if let Some(raw) = &report.scan_limit {
            limits.push(describe_scan_limit(raw));
        }
        if report.duplicate_candidates_truncated() {
            limits.push(format!(
                "Nur die {} größten von {} Kandidaten verglichen",
                thousands(report.duplicate_candidates_retained),
                thousands(report.duplicate_candidates)
            ));
        }
        let shown = report.duplicate_groups.len() as u64;
        if report.result_counts.duplicate_groups > shown {
            limits.push(format!(
                "{} von {} Gruppen angezeigt",
                thousands(shown),
                thousands(report.result_counts.duplicate_groups)
            ));
        }
        Self {
            summary: DuplicateSummary {
                files: report.files,
                bytes: report.bytes,
                candidates: report.duplicate_candidates,
                compared: report.duplicate_candidates_retained,
                groups: shown,
                protected: Vec::new(),
                errors: report.errors,
                suppressed_errors: report.suppressed_errors,
                limits,
            },
            groups: report.duplicate_groups,
            root_error: report.root_error,
        }
    }
}

/// Searches `root` for files with equal contents (at least `min_bytes`).
pub fn find_duplicates(root: &Path, progress: &ReclaimProgress, min_bytes: u64) -> DuplicateReport {
    let limits = FinderLimits {
        min_bytes,
        candidate_text_bytes: candidate_text_budget(),
        threads: crate::analytics::local_scan_threads(),
    };
    find_duplicates_in(
        root,
        progress,
        limits,
        &ProtectedAreas::for_walk(root),
        None,
    )
}

pub(crate) fn find_duplicates_in(
    root: &Path,
    progress: &ReclaimProgress,
    limits: FinderLimits,
    protected: &ProtectedAreas,
    guard: Option<Guard<'_>>,
) -> DuplicateReport {
    let roots = [FinderRoot {
        path: root.to_path_buf(),
        protected: protected.clone(),
    }];
    find_duplicates_in_roots(&roots, progress, limits, guard, &[])
}

/// One search over several roots (the exports of a host): the candidates of
/// every root are compared with each other. Roots must not lie inside each
/// other, or a file would be reported as its own duplicate; `excluded`
/// folders are never entered (a host's own data).
pub(crate) fn find_duplicates_in_roots(
    roots: &[FinderRoot],
    progress: &ReclaimProgress,
    limits: FinderLimits,
    guard: Option<Guard<'_>>,
    excluded: &[std::path::PathBuf],
) -> DuplicateReport {
    find_duplicates_in_roots_with_open(roots, progress, limits, guard, excluded,
        &crate::local_access::DirectoryHandle::open_root_consented)
}

pub(crate) fn find_duplicates_in_roots_with_open(
    roots: &[FinderRoot], progress: &ReclaimProgress, limits: FinderLimits,
    guard: Option<Guard<'_>>, excluded: &[std::path::PathBuf],
    open: &dyn Fn(&Path) -> io::Result<crate::local_access::DirectoryHandle>,
) -> DuplicateReport {
    let limits = FinderLimits {
        min_bytes: limits.min_bytes.max(1),
        ..limits
    };
    let issues = Issues::default();
    let pool = if limits.threads > 1 && crate::analytics::os::parallel_scan_allowed() {
        rayon::ThreadPoolBuilder::new()
            .num_threads(limits.threads)
            .stack_size(crate::analytics::SCAN_THREAD_STACK_BYTES)
            .build()
            .ok()
    } else {
        None
    };
    // A failed pool build stays serial instead of using Rayon's global pool.
    let harvest = Harvest::default();
    let mut handles = Vec::new();
    for root in roots {
        if progress.cancel.load(Ordering::Relaxed) {
            break;
        }
        let handle = match open(&root.path) {
            Ok(handle) => handle,
            Err(error) => {
                issues.failed(std::slice::from_ref(&root.protected), &root.path, true,
                    || format!("{}: {error}", to_fwd(&root.path)));
                continue;
            }
        };
        let root_index = handles.len();
        handles.push((root.path.clone(), handle));
        let walk = Walk {
            root_index,
            handle: &handles[root_index].1,
            root: &root.path,
            progress,
            limits,
            protected: &root.protected,
            guard,
            issues: &issues,
            parallel: pool.is_some(),
            excluded,
            harvest: &harvest,
        };
        match &pool {
            Some(pool) => pool.install(|| walk.run()),
            None => walk.run(),
        }
    }
    let walked = harvest.finish();
    let areas: Vec<ProtectedAreas> = roots.iter().map(|root| root.protected.clone()).collect();
    let compare = Compare {
        pool: pool.as_ref(),
        progress,
        protected: &areas,
        issues: &issues,
        sample_bytes: SAMPLE_BYTES,
        roots: &handles,
    };
    let (groups, compared) = if progress.cancel.load(Ordering::Relaxed) {
        (Vec::new(), 0)
    } else {
        compare_candidates(walked.candidates, &compare)
    };
    let mut limit_lines = Vec::new();
    if let Some(limit) = walked.limit {
        limit_lines.push(limit.describe());
    }
    if walked.dropped > 0 {
        limit_lines.push(format!(
            "Kandidatenspeicher ({} MiB) ausgeschöpft: {} Dateien nicht verglichen",
            limits.candidate_text_bytes / (1024 * 1024),
            thousands(walked.dropped)
        ));
    }
    let (errors, suppressed_errors, root_error, protected) = issues.finish();
    DuplicateReport {
        summary: DuplicateSummary {
            files: progress.files.load(Ordering::Relaxed),
            bytes: progress.bytes.load(Ordering::Relaxed),
            candidates: walked.eligible,
            compared,
            groups: groups.len() as u64,
            protected,
            errors,
            suppressed_errors,
            limits: limit_lines,
        },
        groups,
        root_error,
    }
}

/// Errors and protected omissions shared by the walk and the comparison.
#[derive(Default)]
pub(super) struct Issues {
    errors: Mutex<(Vec<String>, u64)>,
    root_error: Mutex<Option<String>>,
    protected: ProtectedTally,
}

impl Issues {
    /// `at` failed: inside a protected area a counted omission, elsewhere a
    /// bounded error (`text` is only built then).
    pub(super) fn failed(
        &self,
        areas: &[ProtectedAreas],
        at: &Path,
        is_root: bool,
        text: impl FnOnce() -> String,
    ) {
        if let Some(area) = areas.iter().find_map(|areas| areas.area_of(at)) {
            self.protected.omit(&to_fwd(area));
            return;
        }
        let text = text();
        if is_root {
            let mut root = self.root_error.lock().unwrap_or_else(|p| p.into_inner());
            if root.is_none() {
                *root = Some(text.clone());
            }
        }
        let mut errors = self.errors.lock().unwrap_or_else(|p| p.into_inner());
        let (kept, suppressed) = &mut *errors;
        push_bounded_error(kept, suppressed, text);
    }

    pub(super) fn visit(&self, area: &Path) {
        self.protected.visit(&to_fwd(area));
    }

    fn finish(self) -> (Vec<String>, u64, Option<String>, Vec<ProtectedOmission>) {
        let (errors, suppressed) = self.errors.into_inner().unwrap_or_else(|p| p.into_inner());
        let root = self
            .root_error
            .into_inner()
            .unwrap_or_else(|p| p.into_inner());
        (errors, suppressed, root, self.protected.finish())
    }
}
