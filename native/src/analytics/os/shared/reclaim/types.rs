use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

#[derive(Clone)]
pub struct ReclaimOptions {
    pub large_min_bytes: u64,
    pub stale_days: u64,
    pub max_items: usize,
    pub duplicate_min_bytes: u64,
    pub partial_fingerprint_bytes: u64,
}

impl Default for ReclaimOptions {
    fn default() -> Self {
        Self {
            large_min_bytes: 1024 * 1024 * 1024,
            stale_days: 365,
            max_items: 200,
            duplicate_min_bytes: 1024 * 1024,
            partial_fingerprint_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Default)]
pub struct ReclaimProgress {
    pub files: Arc<AtomicU64>,
    pub dirs: Arc<AtomicU64>,
    pub bytes: Arc<AtomicU64>,
    pub fingerprinted: Arc<AtomicU64>,
    pub hashed: Arc<AtomicU64>,
    pub candidates: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReclaimConfidence {
    VerifiedExact,
    HashMatch,
    ReviewSafe,
    RiskyReview,
    NeverAuto,
}

impl ReclaimConfidence {
    pub fn label(self) -> &'static str {
        match self {
            ReclaimConfidence::VerifiedExact => "verifiziert",
            ReclaimConfidence::HashMatch => "Hash-Match",
            ReclaimConfidence::ReviewSafe => "Review sicher",
            ReclaimConfidence::RiskyReview => "Review riskant",
            ReclaimConfidence::NeverAuto => "nie automatisch",
        }
    }

    pub fn quick_selectable(self) -> bool {
        matches!(
            self,
            ReclaimConfidence::VerifiedExact | ReclaimConfidence::ReviewSafe
        )
    }

    pub fn needs_warning(self) -> bool {
        matches!(
            self,
            ReclaimConfidence::RiskyReview | ReclaimConfidence::NeverAuto
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashAlgorithm {
    Md5,
    Sha256,
}

impl HashAlgorithm {
    pub fn label(self) -> &'static str {
        match self {
            HashAlgorithm::Md5 => "MD5",
            HashAlgorithm::Sha256 => "SHA-256",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentHash {
    pub algorithm: HashAlgorithm,
    pub hex: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DuplicateEvidence {
    LocalSha256,
    ProviderMd5,
    AgentMd5,
}

impl DuplicateEvidence {
    pub fn label(self) -> &'static str {
        match self {
            DuplicateEvidence::LocalSha256 => "lokal + SHA-256",
            DuplicateEvidence::ProviderMd5 => "Provider-MD5",
            DuplicateEvidence::AgentMd5 => "Agent-MD5",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReclaimItem {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub is_dir: bool,
    pub reason: String,
    pub confidence: ReclaimConfidence,
    pub backend_id: Option<String>,
}

impl ReclaimItem {
    pub(crate) fn new(path: String, name: String, size: u64, mtime_ms: i64, is_dir: bool) -> Self {
        Self {
            path,
            name,
            size,
            mtime_ms,
            is_dir,
            reason: String::new(),
            confidence: ReclaimConfidence::RiskyReview,
            backend_id: None,
        }
    }

    pub(crate) fn with_reason(
        mut self,
        reason: impl Into<String>,
        confidence: ReclaimConfidence,
    ) -> Self {
        self.reason = reason.into();
        self.confidence = confidence;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub hash: ContentHash,
    pub evidence: DuplicateEvidence,
    pub size: u64,
    pub reclaimable: u64,
    pub items: Vec<ReclaimItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReclaimReport {
    pub root: String,
    pub is_remote: bool,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub large_files: Vec<ReclaimItem>,
    pub stale_files: Vec<ReclaimItem>,
    pub empty_files: Vec<ReclaimItem>,
    pub empty_dirs: Vec<ReclaimItem>,
    pub cleanup: Vec<ReclaimItem>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub errors: Vec<String>,
}

impl ReclaimReport {
    pub fn reclaimable_bytes(&self) -> u64 {
        let dup = self
            .duplicate_groups
            .iter()
            .map(|g| g.reclaimable)
            .sum::<u64>();
        let empty = self.empty_files.iter().map(|i| i.size).sum::<u64>();
        let cleanup = self
            .cleanup
            .iter()
            .filter(|i| i.confidence.quick_selectable())
            .map(|i| i.size)
            .sum::<u64>();
        dup + empty + cleanup
    }

    pub fn prune_paths(&mut self, paths: &[String]) {
        let gone: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        let keep = |i: &ReclaimItem| !gone.contains(i.path.as_str());
        self.large_files.retain(keep);
        self.stale_files.retain(keep);
        self.empty_files.retain(keep);
        self.empty_dirs.retain(keep);
        self.cleanup.retain(keep);
        for g in &mut self.duplicate_groups {
            g.items.retain(keep);
            g.reclaimable = g
                .size
                .saturating_mul(g.items.len().saturating_sub(1) as u64);
        }
        self.duplicate_groups.retain(|g| g.items.len() > 1);
    }
}

#[derive(Clone)]
pub(crate) struct FileCandidate {
    pub path: PathBuf,
    pub item: ReclaimItem,
}
