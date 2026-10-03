//! Messages of a host-side duplicate search (`duplicate_search_v1`,
//! `FsResponse::Duplicates`): heartbeats with the host's progress, the groups
//! in bounded portions (largest reclaimable space first) and the summary.
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};

use crate::analytics::{
    ContentHash, DuplicateEvidence, DuplicateGroup, DuplicateSummary, HashAlgorithm,
    ProtectedOmission, ReclaimConfidence, ReclaimItem, ReclaimPhase, ReclaimProgress,
};

/// Paths of one portion before it is sent (estimate of their JSON).
pub(crate) const GROUP_PORTION_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "m", rename_all = "snake_case")]
pub(crate) enum FsDuplicateMessage {
    Progress { state: FsDuplicateProgress },
    Groups { groups: Vec<FsDuplicateGroup> },
    Done { summary: FsDuplicateSummary },
}

/// Phase of the host's search (`ReclaimPhase`); unknown values read as the walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FsDuplicatePhase {
    Fingerprinting,
    Hashing,
    Grouping,
    #[default]
    #[serde(other)]
    Walking,
}

/// The host's counters; `queued` is the place in the host's queue (0 = runs).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsDuplicateProgress {
    #[serde(default)]
    pub(crate) queued: u32,
    #[serde(default)]
    pub(crate) phase: FsDuplicatePhase,
    #[serde(default)]
    pub(crate) files: u64,
    #[serde(default)]
    pub(crate) dirs: u64,
    #[serde(default)]
    pub(crate) bytes: u64,
    #[serde(default)]
    pub(crate) fingerprinted: u64,
    #[serde(default)]
    pub(crate) hashed: u64,
    #[serde(default)]
    pub(crate) candidates: u64,
    /// Files and bytes the current phase works through, and bytes done.
    #[serde(default)]
    pub(crate) phase_files: u64,
    #[serde(default)]
    pub(crate) phase_bytes: u64,
    #[serde(default)]
    pub(crate) phase_bytes_done: u64,
    /// Folder being read (visible path, shortened).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) current: String,
}

/// Files of equal content (SHA-256 over the whole content).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsDuplicateGroup {
    pub(crate) sha256: String,
    pub(crate) size: u64,
    pub(crate) files: Vec<FsDuplicateFile>,
    /// Further files of this same group follow in the next portion.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) more: bool,
}

fn is_false(value: &bool) -> bool { !*value }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsDuplicateFile {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) mtime_ms: i64,
}

/// `DuplicateSummary` plus the root failure.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsDuplicateSummary {
    #[serde(default)]
    pub(crate) files: u64,
    #[serde(default)]
    pub(crate) bytes: u64,
    #[serde(default)]
    pub(crate) candidates: u64,
    #[serde(default)]
    pub(crate) compared: u64,
    #[serde(default)]
    pub(crate) groups: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) protected: Vec<ProtectedOmission>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) errors: Vec<String>,
    #[serde(default)]
    pub(crate) suppressed_errors: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) limits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) root_error: Option<String>,
}

impl FsDuplicateProgress {
    /// The host side: a snapshot of its search.
    pub(crate) fn of(progress: &ReclaimProgress, queued: u32, current: String) -> Self {
        let load = |counter: &std::sync::atomic::AtomicU64| counter.load(Ordering::Relaxed);
        let (phase_files, phase_bytes, phase_bytes_done) = progress.stage.totals();
        Self {
            queued,
            phase: match progress.stage.phase() {
                ReclaimPhase::Walking => FsDuplicatePhase::Walking,
                ReclaimPhase::Fingerprinting => FsDuplicatePhase::Fingerprinting,
                ReclaimPhase::Hashing => FsDuplicatePhase::Hashing,
                ReclaimPhase::Grouping => FsDuplicatePhase::Grouping,
            },
            files: load(&progress.files),
            dirs: load(&progress.dirs),
            bytes: load(&progress.bytes),
            fingerprinted: load(&progress.fingerprinted),
            hashed: load(&progress.hashed),
            candidates: load(&progress.candidates),
            phase_files,
            phase_bytes,
            phase_bytes_done,
            current,
        }
    }

    /// The client side: the host's state into the caller's progress.
    pub(crate) fn apply(&self, progress: &ReclaimProgress) {
        progress.files.store(self.files, Ordering::Relaxed);
        progress.dirs.store(self.dirs, Ordering::Relaxed);
        progress.bytes.store(self.bytes, Ordering::Relaxed);
        progress
            .fingerprinted
            .store(self.fingerprinted, Ordering::Relaxed);
        progress.hashed.store(self.hashed, Ordering::Relaxed);
        progress
            .candidates
            .store(self.candidates, Ordering::Relaxed);
        let phase = match self.phase {
            FsDuplicatePhase::Walking => ReclaimPhase::Walking,
            FsDuplicatePhase::Fingerprinting => ReclaimPhase::Fingerprinting,
            FsDuplicatePhase::Hashing => ReclaimPhase::Hashing,
            FsDuplicatePhase::Grouping => ReclaimPhase::Grouping,
        };
        progress.stage.mirror(
            phase,
            (self.phase_files, self.phase_bytes, self.phase_bytes_done),
            &self.current,
        );
    }
}

impl FsDuplicateGroup {
    /// The host side: one group with the paths the peer sees.
    pub(crate) fn of(group: &DuplicateGroup, visible: impl Fn(&str) -> String) -> Self {
        Self {
            sha256: group.hash.hex.clone(),
            size: group.size,
            more: false,
            files: group
                .items
                .iter()
                .map(|item| FsDuplicateFile {
                    path: visible(&item.path),
                    mtime_ms: item.mtime_ms,
                })
                .collect(),
        }
    }

    /// JSON bytes of this group (estimate).
    pub(crate) fn wire_bytes(&self) -> usize {
        self.files
            .iter()
            .map(|file| file.path.len() + 48)
            .sum::<usize>()
            + 128
    }

    /// The client side, in the shape of the local search: newest first, every
    /// file a candidate whose content the host compared byte for byte.
    pub(crate) fn into_group(self) -> DuplicateGroup {
        let size = self.size;
        let mut items: Vec<ReclaimItem> = self
            .files
            .into_iter()
            .map(|file| {
                let name = file.path.rsplit('/').next().unwrap_or_default().to_string();
                ReclaimItem::new(file.path, name, size, file.mtime_ms, false)
                    .with_reason("Duplikat", ReclaimConfidence::HashMatch)
            })
            .collect();
        items.sort_by(|left, right| {
            right
                .mtime_ms
                .cmp(&left.mtime_ms)
                .then_with(|| left.path.cmp(&right.path))
        });
        DuplicateGroup {
            hash: ContentHash {
                algorithm: HashAlgorithm::Sha256,
                hex: self.sha256,
            },
            // The host's own SHA-256 over its local copies.
            evidence: DuplicateEvidence::LocalSha256,
            size,
            reclaimable: size.saturating_mul(items.len().saturating_sub(1) as u64),
            items,
        }
    }
}

impl FsDuplicateSummary {
    pub(crate) fn fit_wire(&mut self) -> std::io::Result<()> {
        let bounded = |text: &mut String, max| {
            let mut start = text.len().saturating_sub(max);
            while !text.is_char_boundary(start) { start += 1; }
            if start > 0 { *text = format!("…{}", &text[start..]); }
        };
        for text in self.errors.iter_mut().chain(&mut self.limits) { bounded(text, 4093); }
        if let Some(text) = &mut self.root_error { bounded(text, 4093); }
        loop {
            if serde_json::to_vec(self).map_err(std::io::Error::other)?.len() <= GROUP_PORTION_BYTES { return Ok(()); }
            if self.errors.pop().is_some() { self.suppressed_errors = self.suppressed_errors.saturating_add(1); continue; }
            let longest = self.limits.iter_mut().max_by_key(|text| text.len());
            if let Some(text) = longest.filter(|text| text.len() > 128) {
                let max = text.len() / 2;
                bounded(text, max);
                continue;
            }
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"Geschützte Duplikat-Metadaten überschreiten das Drahtformat"));
        }
    }

    pub(crate) fn of(summary: DuplicateSummary, root_error: Option<String>) -> Self {
        Self {
            files: summary.files,
            bytes: summary.bytes,
            candidates: summary.candidates,
            compared: summary.compared,
            groups: summary.groups,
            protected: summary.protected,
            errors: summary.errors,
            suppressed_errors: summary.suppressed_errors,
            limits: summary.limits,
            root_error,
        }
    }

    pub(crate) fn into_summary(self) -> (DuplicateSummary, Option<String>) {
        (
            DuplicateSummary {
                files: self.files,
                bytes: self.bytes,
                candidates: self.candidates,
                compared: self.compared,
                groups: self.groups,
                protected: self.protected,
                errors: self.errors,
                suppressed_errors: self.suppressed_errors,
                limits: self.limits,
            },
            self.root_error,
        )
    }
}
