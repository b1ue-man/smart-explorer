//! What one transfer moves: endpoints, selection, layout and conflict policy.
//! The engine derives everything else (discovery, folders, concurrency).
use super::types::TransferKind;
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

    /// Both sides are the same connection (server-side copies apply).
    pub fn same_as(&self, other: &Endpoint) -> bool {
        match (self, other) {
            (Self::Local, Self::Local) => true,
            (Self::Remote(one), Self::Remote(two)) => Arc::ptr_eq(one, two),
            _ => false,
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
    /// Explicit files as (absolute source, relative destination) pairs, for
    /// example the matching files of a filtered recursive view. Only their
    /// parent folders are created.
    Pairs(Vec<(String, String)>),
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
    /// silently): moving from or to a remote location.
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
        Ok(())
    }
}
