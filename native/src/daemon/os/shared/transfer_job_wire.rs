//! Wire of the worker's job service (Z-P2, 2026-10-28). A transfer job with
//! a Share side runs inside the worker that owns the Share node: the app,
//! `se` and `se mcp` send the job (`Start`), the worker answers `Started`
//! and then streams progress and exactly one `Done` on the same connection.
//! `Cancel` stops a running job from any connection with the daemon token.
//! `Hello` names the service versions; a worker that predates the service
//! refuses the unknown request and the client keeps the proxy path.
//!
//! The job travels without live handles: local sides as `Local`, Share
//! sides as the device the worker opens itself, with the identity the app's
//! handle persists jobs under (`transfer::remember_identity`).
use crate::share::PeerOpenTarget;
use crate::transfer::{
    FolderConflictPolicy, JobItems, JobView, Layout, RecoveryMode, ResolvedRoot, StoredFilter,
    TransferIssue, TransferProgress,
};
use crate::types::{Conflict, CopyMode, FilterDef};
use serde::{Deserialize, Serialize};

/// The job service's label in the `Hello` reply.
pub(crate) const TRANSFER_JOBS_V1: &str = "transfer-jobs-v1";
/// A worker with this label also decides the file rule "neuere ersetzen
/// ältere" (`Conflict::Update`, file policy code 3, `merge_update` of the
/// folder policy). A job that uses the rule goes only to such a worker; for
/// an older one the client keeps the proxy path, which runs the job itself.
pub(crate) const TRANSFER_UPDATE_V1: &str = "transfer-update-v1";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TransferIpcRequest {
    /// Which versions of the job service the worker speaks.
    Hello,
    /// Runs `job` in the worker.
    Start { job: Box<JobWire> },
    /// Stops the running job `id` (the id `Started` named).
    Cancel { id: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TransferIpcReply {
    Hello {
        capabilities: Vec<String>,
    },
    /// The job runs under this id (its own id, or one the worker gave a job
    /// without an id).
    Started {
        id: String,
    },
    Progress {
        progress: Box<TransferProgress>,
    },
    /// The job's terminal message (`TransferMsg::Done`).
    Done {
        progress: Box<TransferProgress>,
        errors: Vec<String>,
        canceled: bool,
        issues: Vec<TransferIssue>,
        roots: Vec<ResolvedRoot>,
    },
    Cancelled {
        found: bool,
    },
}

/// One side of a job on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum EndpointWire {
    Local,
    /// A Share device the worker opens; `namespace` and `state` are the
    /// identities the app's handle records jobs under.
    Share {
        target: PeerOpenTarget,
        namespace: String,
        state: String,
    },
}

/// A job as `JobView` describes it, without live handles.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct JobWire {
    pub(crate) id: Option<String>,
    pub(crate) recoverable: bool,
    /// `RecoveryMode::Resume`: the job reopens its record.
    pub(crate) resume_record: bool,
    pub(crate) source: EndpointWire,
    pub(crate) target: EndpointWire,
    pub(crate) target_dir: String,
    pub(crate) items: JobItems,
    pub(crate) layout: Layout,
    pub(crate) filter: Option<StoredFilter>,
    /// 0 rename, 1 overwrite, 2 skip, 3 update (`TRANSFER_UPDATE_V1`).
    pub(crate) conflict: u8,
    pub(crate) folders: FolderConflictPolicy,
    pub(crate) moving: bool,
    pub(crate) source_label: String,
    pub(crate) target_label: String,
    pub(crate) resume: Option<Vec<ResolvedRoot>>,
    /// The job keeps running when the client's connection ends (a crash of
    /// the app): only jobs with a crash log, whose end the app learns again
    /// from its recovery list. Every other job ends with its client.
    pub(crate) survives_client: bool,
}

impl JobWire {
    /// `view` with its sides as `source` and `target`.
    pub(crate) fn of(view: &JobView<'_>, source: EndpointWire, target: EndpointWire) -> Self {
        Self {
            id: view.id.map(str::to_string),
            recoverable: view.recoverable,
            resume_record: view.recovery == RecoveryMode::Resume,
            source,
            target,
            target_dir: view.target_dir.to_string(),
            items: view.items.clone(),
            layout: view.layout,
            filter: view.filter.map(StoredFilter::of),
            conflict: conflict_code(view.conflict),
            folders: view.folders.clone(),
            moving: view.mode == CopyMode::Move,
            source_label: view.source_label.to_string(),
            target_label: view.target_label.to_string(),
            resume: view.resume.map(<[ResolvedRoot]>::to_vec),
            survives_client: view.recoverable && view.id.is_some(),
        }
    }

    pub(crate) fn recovery(&self) -> RecoveryMode {
        if self.resume_record {
            RecoveryMode::Resume
        } else {
            RecoveryMode::New
        }
    }

    pub(crate) fn mode(&self) -> CopyMode {
        if self.moving {
            CopyMode::Move
        } else {
            CopyMode::Copy
        }
    }

    /// The file conflict policy and the filter, checked.
    pub(crate) fn decoded(&self) -> Result<(Conflict, Option<(FilterDef, String)>), String> {
        let conflict = match self.conflict {
            0 => Conflict::Rename,
            1 => Conflict::Overwrite,
            2 => Conflict::Skip,
            3 => Conflict::Update,
            _ => return Err("Unbekannte Dateikonfliktregel im Auftrag".into()),
        };
        let filter = self
            .filter
            .as_ref()
            .map(StoredFilter::restore)
            .transpose()?;
        Ok((conflict, filter))
    }
}

fn conflict_code(conflict: Conflict) -> u8 {
    match conflict {
        Conflict::Rename => 0,
        Conflict::Overwrite => 1,
        Conflict::Skip => 2,
        Conflict::Update => 3,
    }
}

impl JobWire {
    /// Whether the job needs a worker that knows `TRANSFER_UPDATE_V1`.
    pub(crate) fn needs_update_rule(&self) -> bool {
        self.conflict == 3 || self.folders.merge_update
    }
}
