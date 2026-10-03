//! Duplicate candidates of a remote walk and their groups. Every file of at
//! least the minimum size is a candidate; the candidate memory (path text)
//! bounds how many are kept, the largest first – never a fixed count. Files
//! with a hash from the backend (provider or agent MD5) group by it; files
//! without one are compared by content (`backend_compare.rs`).
use std::cmp::{Ordering as CmpOrdering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::atomic::Ordering;

use super::backend_compare::compare_by_content;
use super::finder::{DuplicateReport, DuplicateSummary};
use super::retention::compare_group;
use super::types::{
    ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimItem, ReclaimProgress,
};
use crate::vfs::Backend;

/// One candidate and its backend hash, when the backend has one.
pub(super) struct Candidate {
    pub(super) item: ReclaimItem,
    pub(super) hash: Option<(String, DuplicateEvidence)>,
}

impl Candidate {
    fn text_bytes(&self) -> u64 {
        let id = self.item.backend_id.as_ref().map_or(0, String::len);
        (self.item.path.len() + self.item.name.len() + id) as u64
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == CmpOrdering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

/// Larger files rank higher; equal sizes by path, so eviction is stable.
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.item
            .size
            .cmp(&other.item.size)
            .then_with(|| other.item.path.cmp(&self.item.path))
    }
}

/// The candidates of one walk: the largest ones whose path text fits
/// `budget`, plus how many were seen and how many had to be left out.
pub(super) struct Candidates {
    kept: BinaryHeap<Reverse<Candidate>>,
    text_bytes: u64,
    budget: u64,
    seen: u64,
    dropped: u64,
}

impl Candidates {
    pub(super) fn new(budget: u64) -> Self {
        Self {
            kept: BinaryHeap::new(),
            text_bytes: 0,
            budget,
            seen: 0,
            dropped: 0,
        }
    }

    /// Keeps `candidate`; while the kept text exceeds the budget the smallest
    /// candidate goes (possibly this one).
    pub(super) fn offer(&mut self, candidate: Candidate) {
        self.seen = self.seen.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(candidate.text_bytes());
        self.kept.push(Reverse(candidate));
        while self.text_bytes > self.budget {
            let Some(Reverse(smallest)) = self.kept.pop() else {
                break;
            };
            self.text_bytes = self.text_bytes.saturating_sub(smallest.text_bytes());
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Files of at least the minimum size, kept or not.
    pub(super) fn seen(&self) -> u64 {
        self.seen
    }

    pub(super) fn dropped(&self) -> u64 {
        self.dropped
    }

    pub(super) fn len(&self) -> u64 {
        self.kept.len() as u64
    }

    pub(super) fn take(&mut self) -> Vec<Candidate> {
        self.text_bytes = 0;
        std::mem::take(&mut self.kept)
            .into_vec()
            .into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect()
    }
}

/// Groups found among the candidates: the first `limit` (largest
/// reclaimable space first), how many there are in total, how many
/// candidates were compared and the reads that failed.
pub(super) struct Found {
    pub(super) groups: Vec<DuplicateGroup>,
    pub(super) total_groups: u64,
    pub(super) compared: u64,
    pub(super) errors: Vec<String>,
}

pub(super) fn duplicate_groups(
    backend: &dyn Backend,
    candidates: Vec<Candidate>,
    progress: &ReclaimProgress,
    limit: usize,
) -> Found {
    let mut by_hash: BTreeMap<(u64, String, DuplicateEvidence), Vec<ReclaimItem>> = BTreeMap::new();
    let mut unhashed = Vec::new();
    let mut compared = 0u64;
    for candidate in candidates {
        match candidate.hash {
            Some((hash, evidence)) => {
                compared = compared.saturating_add(1);
                by_hash
                    .entry((candidate.item.size, hash, evidence))
                    .or_default()
                    .push(candidate.item);
            }
            None => unhashed.push(candidate.item),
        }
    }
    let mut groups = Vec::new();
    for ((size, hex, evidence), mut items) in by_hash {
        if items.len() < 2 {
            continue;
        }
        items.sort_by(|left, right| {
            right
                .mtime_ms
                .cmp(&left.mtime_ms)
                .then_with(|| left.path.cmp(&right.path))
        });
        progress
            .candidates
            .fetch_add(items.len() as u64, Ordering::Relaxed);
        groups.push(DuplicateGroup {
            hash: ContentHash {
                algorithm: HashAlgorithm::Md5,
                hex,
            },
            evidence,
            size,
            reclaimable: size.saturating_mul(items.len().saturating_sub(1) as u64),
            items,
        });
    }
    let mut errors = Vec::new();
    if !unhashed.is_empty() && !progress.cancel.load(Ordering::Relaxed) {
        let content = compare_by_content(backend, unhashed, progress);
        compared = compared.saturating_add(content.compared);
        groups.extend(content.groups);
        errors = content.errors;
    }
    groups.sort_by(compare_group);
    let total_groups = groups.len() as u64;
    groups.truncate(limit);
    Found {
        groups,
        total_groups,
        compared,
        errors,
    }
}

/// Whether the device that stores `root` searches its duplicates itself.
pub(super) fn host_searches(backend: &crate::vfs::BackendHandle, root: &str) -> bool {
    crate::vfs::supports_duplicate_search(&**backend, root).unwrap_or(false)
}

/// The storing host's own duplicate search; a failure is the root's error.
pub(super) fn host_report(
    backend: &crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    min_bytes: u64,
) -> Option<DuplicateReport> {
    let counters = [&progress.files, &progress.dirs, &progress.bytes, &progress.fingerprinted,
        &progress.hashed, &progress.candidates];
    let before = counters.map(|counter| counter.load(Ordering::Relaxed));
    match crate::vfs::find_duplicates(&**backend, root, min_bytes, progress) {
        Ok(None) => {
            for (counter, value) in counters.into_iter().zip(before) { counter.store(value, Ordering::Relaxed); }
            progress.stage.begin(super::types::ReclaimPhase::Walking, 0, 0);
            None
        }
        Ok(report) => report,
        Err(error) => Some(failed_report(format!("{root}: {error}"))),
    }
}

fn failed_report(error: String) -> DuplicateReport {
    DuplicateReport {
        groups: Vec::new(),
        summary: DuplicateSummary::default(),
        root_error: Some(error),
    }
}
