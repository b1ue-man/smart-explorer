//! Every copy entry point of the explorer (paste, drop, "Kopieren nach…",
//! "Herunterladen nach…", "Für andere Programme bereitstellen") becomes an
//! engine job here, so each combination of local and remote sides behaves the
//! same: numbered names instead of replacing, moving only between local
//! folders, and never a folder copied into itself.
use crate::transfer::{Endpoint, JobItems, Layout, PairItem, ResolvedRoot, TransferJob};
use crate::types::{Conflict, CopyMode, FilterDef};
use crate::vfs::BackendHandle;

/// Refusal shown when a move involves a remote side (sources stay untouched).
pub(in crate::app) const REMOTE_MOVE_REFUSED: &str =
    "Verschieben von/zu Remote wird nicht unterstützt. Bitte kopieren; die Quelldateien bleiben unverändert.";

/// One side of a transfer: the local filesystem or one connection.
#[derive(Clone)]
pub(in crate::app) struct TransferPlace {
    pub(in crate::app) endpoint: Endpoint,
    /// Connection label of a remote side; empty for the local filesystem.
    pub(in crate::app) label: String,
}

impl TransferPlace {
    pub(in crate::app) fn local() -> Self {
        Self {
            endpoint: Endpoint::Local,
            label: String::new(),
        }
    }

    pub(in crate::app) fn remote(backend: BackendHandle, label: impl Into<String>) -> Self {
        Self {
            endpoint: Endpoint::Remote(backend),
            label: label.into(),
        }
    }

    pub(in crate::app) fn is_local(&self) -> bool {
        self.endpoint.is_local()
    }

    pub(in crate::app) fn backend(&self) -> Option<&BackendHandle> {
        self.endpoint.backend()
    }

    /// Both places address one namespace (two handles to one account count
    /// as one; equal paths on different remotes never do).
    pub(in crate::app) fn same_place(&self, other: &Self) -> bool {
        self.endpoint.same_namespace(&other.endpoint)
    }

    /// How a folder of this side is shown in lists and notices.
    pub(in crate::app) fn describe(&self, path: &str) -> String {
        if self.is_local() || self.label.trim().is_empty() {
            path.to_string()
        } else {
            format!("{}: {path}", self.label.trim())
        }
    }
}

/// What a copy takes along: whole selected entries or explicit files, with
/// the view's filter for entries below selected folders.
#[derive(Clone)]
pub(in crate::app) struct TransferSelection {
    pub(in crate::app) source: TransferPlace,
    pub(in crate::app) items: JobItems,
    pub(in crate::app) filter: Option<(FilterDef, String)>,
}

impl TransferSelection {
    /// Whole entries; each lands under its path relative to `base`, or under
    /// its own name without a base. The base is kept without a trailing
    /// slash (`C:` for a drive root, empty for `/`), the form relative
    /// destination paths are cut from.
    pub(in crate::app) fn roots(
        source: TransferPlace,
        paths: Vec<String>,
        base: Option<String>,
    ) -> Self {
        let base = base.map(|base| base.trim_end_matches('/').to_string());
        Self {
            source,
            items: JobItems::Roots { paths, base },
            filter: None,
        }
    }

    /// Explicit files with their destination paths (a filtered view's
    /// snapshot); only their parent folders are created.
    pub(in crate::app) fn pairs(source: TransferPlace, pairs: Vec<PairItem>) -> Self {
        Self {
            source,
            items: JobItems::Pairs(pairs),
            filter: None,
        }
    }

    pub(in crate::app) fn with_filter(mut self, filter: Option<(FilterDef, String)>) -> Self {
        self.filter = filter;
        self
    }

    pub(in crate::app) fn count(&self) -> usize {
        self.items.len()
    }

    /// Where the selection comes from, as lists and notices show it.
    pub(in crate::app) fn describe_source(&self) -> String {
        self.source.describe(&self.origin())
    }

    /// The folder the selection comes from.
    fn origin(&self) -> String {
        let first = match &self.items {
            JobItems::Roots { paths, .. } => paths.first().map(String::as_str),
            JobItems::Pairs(pairs) => pairs.first().map(|pair| pair.source.as_str()),
        };
        first.map(parent_dir).unwrap_or_default()
    }
}

/// Paste, drop and "Herunterladen nach…": occupied names become "Name (2)",
/// local folders merge and remote folders get a new name; nothing is replaced.
pub(in crate::app) fn paste_job(
    selection: &TransferSelection,
    target: &TransferPlace,
    target_dir: &str,
    mode: CopyMode,
) -> Result<TransferJob, String> {
    build_job(
        selection,
        target,
        target_dir,
        Layout::Tree,
        Conflict::Rename,
        mode,
    )
}

/// "Kopieren/Verschieben nach…" into a local folder with the dialog's
/// conflict policy; without structure every file lands directly in the target.
pub(in crate::app) fn dialog_job(
    selection: &TransferSelection,
    target_dir: &str,
    preserve_structure: bool,
    conflict: Conflict,
    mode: CopyMode,
) -> Result<TransferJob, String> {
    let layout = if preserve_structure {
        Layout::Tree
    } else {
        Layout::Flatten
    };
    build_job(
        selection,
        &TransferPlace::local(),
        target_dir,
        layout,
        conflict,
        mode,
    )
}

/// "Fehlende übertragen": the same selection into the target roots the earlier
/// run resolved; present files of equal size are skipped and nothing is
/// replaced or copied again under a new number.
pub(in crate::app) fn resume_job(job: &TransferJob, roots: &[ResolvedRoot]) -> TransferJob {
    let mut resumed = job.clone();
    resumed.resume = Some(roots.to_vec());
    resumed
}

fn build_job(
    selection: &TransferSelection,
    target: &TransferPlace,
    target_dir: &str,
    layout: Layout,
    conflict: Conflict,
    mode: CopyMode,
) -> Result<TransferJob, String> {
    if target_dir.trim().is_empty() {
        return Err("Kein Zielordner gewählt".to_string());
    }
    if mode == CopyMode::Move && !(selection.source.is_local() && target.is_local()) {
        return Err(REMOTE_MOVE_REFUSED.to_string());
    }
    let job = TransferJob {
        source: selection.source.endpoint.clone(),
        target: target.endpoint.clone(),
        target_dir: target_dir.to_string(),
        items: selection.items.clone(),
        layout,
        filter: selection.filter.clone(),
        conflict,
        mode,
        source_label: selection.describe_source(),
        target_label: target.describe(target_dir),
        resume: None,
    };
    job.validate()?;
    Ok(job)
}

/// Parent folder of a forward-slash path (`/` for top-level entries).
pub(in crate::app) fn parent_dir(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        // Keep a drive root spelled as a folder (`C:/`).
        Some((parent, _)) if parent.len() == 2 && parent.ends_with(':') => format!("{parent}/"),
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "transfer_route_tests.rs"]
mod tests;
