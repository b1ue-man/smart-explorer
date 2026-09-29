//! A transfer job as the engine reads it: the same fields as `TransferJob`,
//! with borrowed endpoints, so the lane (owned handles) and the older worker
//! functions (borrowed backends) run one engine.
use super::super::job::{path_within, Endpoint, JobItems, Layout, TransferJob};
use super::super::types::{ResolvedRoot, TransferKind};
use crate::types::{Conflict, CopyMode, FilterDef};
use crate::vfs::Backend;

/// One side of a job.
#[derive(Clone, Copy)]
pub(crate) enum Side<'a> {
    Local,
    Remote(&'a dyn Backend),
}

impl<'a> Side<'a> {
    pub(crate) fn of(endpoint: &'a Endpoint) -> Self {
        match endpoint {
            Endpoint::Local => Side::Local,
            Endpoint::Remote(backend) => Side::Remote(&**backend),
        }
    }

    pub(crate) fn is_local(self) -> bool {
        matches!(self, Side::Local)
    }

    pub(crate) fn backend(self) -> Option<&'a dyn Backend> {
        match self {
            Side::Remote(backend) => Some(backend),
            Side::Local => None,
        }
    }

    /// Same namespace: the local filesystem, or one remote account reached
    /// through one or two handles (see `Endpoint::same_namespace`).
    pub(crate) fn same_namespace(self, other: Side<'_>) -> bool {
        match (self, other) {
            (Side::Local, Side::Local) => true,
            (Side::Remote(one), Side::Remote(two)) => {
                std::ptr::addr_eq(one, two) || one.namespace_identity() == two.namespace_identity()
            }
            _ => false,
        }
    }

    pub(crate) fn case_sensitive(self, root: &str) -> bool {
        match self {
            Side::Local => true,
            Side::Remote(backend) => backend.case_sensitive_paths(root),
        }
    }
}

/// Everything one engine run needs to know about its job.
#[derive(Clone, Copy)]
pub(crate) struct JobView<'a> {
    pub source: Side<'a>,
    pub target: Side<'a>,
    pub target_dir: &'a str,
    pub items: &'a JobItems,
    pub layout: Layout,
    pub filter: Option<&'a (FilterDef, String)>,
    pub conflict: Conflict,
    pub mode: CopyMode,
    pub source_label: &'a str,
    pub target_label: &'a str,
    pub resume: Option<&'a [ResolvedRoot]>,
}

impl<'a> JobView<'a> {
    pub(crate) fn of(job: &'a TransferJob) -> Self {
        Self {
            source: Side::of(&job.source),
            target: Side::of(&job.target),
            target_dir: &job.target_dir,
            items: &job.items,
            layout: job.layout,
            filter: job.filter.as_ref(),
            conflict: job.conflict,
            mode: job.mode,
            source_label: &job.source_label,
            target_label: &job.target_label,
            resume: job.resume.as_deref(),
        }
    }

    pub(crate) fn kind(&self) -> TransferKind {
        match (self.source, self.target, self.mode) {
            (Side::Local, Side::Local, CopyMode::Move) => TransferKind::Move,
            (Side::Local, Side::Local, CopyMode::Copy) => TransferKind::Local,
            (Side::Local, Side::Remote(_), _) => TransferKind::Upload,
            (Side::Remote(_), Side::Local, _) => TransferKind::Download,
            (Side::Remote(_), Side::Remote(_), _) => TransferKind::RemoteCopy,
        }
    }

    /// The checks of `TransferJob::validate` on a borrowed job: no move from
    /// or to a remote location, something selected, no folder copied into
    /// itself when both sides share a namespace.
    pub(crate) fn validate(&self) -> Result<(), String> {
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
            return Err(inside_source_message(source));
        }
        Ok(())
    }

    fn source_containing_target(&self) -> Option<&'a str> {
        if !self.source.same_namespace(self.target) {
            return None;
        }
        let JobItems::Roots { paths, .. } = self.items else {
            return None;
        };
        let case_sensitive = self.target.case_sensitive(self.target_dir);
        paths
            .iter()
            .map(String::as_str)
            .find(|source| path_within(self.target_dir, source, case_sensitive))
    }

    /// Local sources may name files with `\` on Unix (an ordinary character
    /// there); everything that reaches another system may not.
    pub(crate) fn allow_backslash(&self) -> bool {
        self.source.is_local()
            && self.target.is_local()
            && super::super::platform::backslash_is_name_char()
    }
}

pub(crate) fn inside_source_message(source: &str) -> String {
    format!(
        "Das Ziel liegt in einer der Quellen ({source}); ein Ordner kann nicht in sich selbst kopiert werden."
    )
}
