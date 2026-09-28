//! What one transfer moves: endpoints, selection, layout and conflict policy.
//! The engine derives everything else (discovery, folders, concurrency).
use super::types::{ResolvedRoot, TransferKind};
use crate::types::{Conflict, CopyMode, FilterDef};
use crate::vfs::BackendHandle;
use std::sync::Arc;

/// One side of a transfer.
#[derive(Clone)]
pub enum Endpoint {
    /// The local filesystem; paths use forward slashes.
    Local,
    /// A connected backend (SFTP, FTP, WebDAV, Drive, SMB, Share, ZIP …).
    Remote(BackendHandle),
}

impl Endpoint {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    pub fn backend(&self) -> Option<&BackendHandle> {
        match self {
            Self::Remote(backend) => Some(backend),
            Self::Local => None,
        }
    }

    /// Both sides address the same namespace: the local filesystem, or one
    /// remote account reached through one or two handles. Equal paths are
    /// then the same location (server-side copies apply); equal relative
    /// paths on different remotes never are.
    pub fn same_namespace(&self, other: &Endpoint) -> bool {
        match (self, other) {
            (Self::Local, Self::Local) => true,
            (Self::Remote(one), Self::Remote(two)) => {
                Arc::ptr_eq(one, two) || one.namespace_identity() == two.namespace_identity()
            }
            _ => false,
        }
    }

    /// Whether paths of this side distinguish upper and lower case below
    /// `root`. Local paths are compared exactly here; the engine checks
    /// local targets again on canonical paths.
    pub fn case_sensitive(&self, root: &str) -> bool {
        match self {
            Self::Local => true,
            Self::Remote(backend) => backend.case_sensitive_paths(root),
        }
    }
}

/// One explicit file of a `Pairs` selection with what the view that produced
/// it already knows, so the engine needs no stat per file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairItem {
    /// Absolute source path.
    pub source: String,
    /// Destination path relative to the target folder.
    pub rel: String,
    /// `None` when the producer did not know it; the engine then asks.
    pub size: Option<u64>,
    pub mtime_ms: i64,
    /// Provider id (Drive) so exactly this file is read.
    pub id: Option<String>,
}

impl PairItem {
    /// A pair without known metadata.
    pub fn new(source: impl Into<String>, rel: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            rel: rel.into(),
            size: None,
            mtime_ms: 0,
            id: None,
        }
    }
}

/// The selection a job copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobItems {
    /// Whole selected entries (files or folders). Each lands below the target
    /// folder under its path relative to `base`, or under its own name when
    /// `base` is `None`.
    Roots {
        paths: Vec<String>,
        base: Option<String>,
    },
    /// Explicit files, for example the matching files of a filtered recursive
    /// view. Only their parent folders are created.
    Pairs(Vec<PairItem>),
}

impl JobItems {
    pub fn len(&self) -> usize {
        match self {
            Self::Roots { paths, .. } => paths.len(),
            Self::Pairs(pairs) => pairs.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// How destination paths are formed below the target folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Keep the folder structure below each selected entry.
    #[default]
    Tree,
    /// Every file directly in the target folder (copy dialog without structure).
    Flatten,
}

/// One transfer request as the engine executes it.
#[derive(Clone)]
pub struct TransferJob {
    pub source: Endpoint,
    pub target: Endpoint,
    /// Destination folder in the target's path form (forward slashes).
    pub target_dir: String,
    pub items: JobItems,
    pub layout: Layout,
    /// Filter for entries below selected folders, with the root prefix its
    /// relative paths start at. Selected files always pass.
    pub filter: Option<(FilterDef, String)>,
    /// Name conflicts at local targets follow this policy; remote targets
    /// always keep both under a numbered name and never replace.
    pub conflict: Conflict,
    /// `Move` is valid only between local folders.
    pub mode: CopyMode,
    /// Short labels for progress displays.
    pub source_label: String,
    pub target_label: String,
    /// "Transfer missing files" of an earlier run: selected entries land in
    /// the target roots that run resolved (no new numbered copies), files
    /// already present at the target are skipped and nothing is replaced.
    pub resume: Option<Vec<ResolvedRoot>>,
}

impl TransferJob {
    pub fn kind(&self) -> TransferKind {
        match (&self.source, &self.target, self.mode) {
            (Endpoint::Local, Endpoint::Local, CopyMode::Move) => TransferKind::Move,
            (Endpoint::Local, Endpoint::Local, CopyMode::Copy) => TransferKind::Local,
            (Endpoint::Local, Endpoint::Remote(_), _) => TransferKind::Upload,
            (Endpoint::Remote(_), Endpoint::Local, _) => TransferKind::Download,
            (Endpoint::Remote(_), Endpoint::Remote(_), _) => TransferKind::RemoteCopy,
        }
    }

    /// Rejects combinations the engine does not perform (and never degrades
    /// silently): moving from or to a remote location, and copying a folder
    /// into itself, which would copy its own copy again and again.
    pub fn validate(&self) -> Result<(), String> {
        if self.mode == CopyMode::Move && !(self.source.is_local() && self.target.is_local()) {
            return Err(
                "Verschieben von/zu Remote wird nicht unterstützt. Bitte kopieren; die Quelldateien bleiben unverändert."
                    .to_string(),
            );
        }
        if self.items.is_empty() {
            return Err("Nichts zum Übertragen ausgewählt".to_string());
        }
        if let Some(source) = self.source_containing_target() {
            return Err(format!(
                "Das Ziel liegt in einer der Quellen ({source}); ein Ordner kann nicht in sich selbst kopiert werden."
            ));
        }
        Ok(())
    }

    /// The selected entry that is the target folder or holds it, when both
    /// sides address the same namespace.
    pub fn source_containing_target(&self) -> Option<&str> {
        if !self.source.same_namespace(&self.target) {
            return None;
        }
        let JobItems::Roots { paths, .. } = &self.items else {
            return None;
        };
        let case_sensitive = self.target.case_sensitive(&self.target_dir);
        paths
            .iter()
            .map(String::as_str)
            .find(|source| path_within(&self.target_dir, source, case_sensitive))
    }
}

/// Whether `inner` is `outer` or lies below it (forward-slash paths).
pub fn path_within(inner: &str, outer: &str, case_sensitive: bool) -> bool {
    let normalize = |path: &str| {
        let trimmed = path.trim_end_matches('/');
        if case_sensitive {
            trimmed.to_string()
        } else {
            trimmed.to_lowercase()
        }
    };
    let (inner, outer) = (normalize(inner), normalize(outer));
    outer.is_empty()
        || inner == outer
        || inner
            .strip_prefix(&outer)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::path_within;

    #[test]
    fn transfer_engine_task_path_within_matches_components_only() {
        assert!(path_within("/a/b", "/a/b", true));
        assert!(path_within("/a/b/c", "/a/b/", true));
        assert!(!path_within("/a/bc", "/a/b", true));
        assert!(!path_within("/a", "/a/b", true));
        assert!(path_within("/anything", "/", true));
        assert!(path_within("C:/Data/x", "c:/data", false));
        assert!(!path_within("C:/Data/x", "c:/data", true));
    }
}
