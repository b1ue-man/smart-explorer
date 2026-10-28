//! What a host already read about the files it serves, publishes or closes
//! (Share `evidence_v1`) and folders created in bulk (`create_dirs_v1`). The
//! transfer engine records this evidence instead of asking again with one
//! request per file. A backend without these extensions keeps every caller on
//! its separate requests, unchanged.
use std::io::{self, Read};

use super::{Backend, BatchPut, VfsResult};

/// Metadata of one regular file as its host read it: what `stat` and
/// `vfs::file_identity` of this backend would answer for `path` then. A host
/// reports evidence only for a regular file (no folder, link or special
/// file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEvidence {
    pub path: String,
    /// `vfs::file_identity` of the file; `None` where the provider has none.
    pub object: Option<String>,
    pub id: Option<String>,
    pub size: u64,
    pub mtime_ms: i64,
}

/// A private stage as its host reported it when the write was committed,
/// and whether `destination` (the path the stage is meant for, named by its
/// stage name) existed at that moment; `None` = not asked or unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageEvidence {
    pub stage: FileEvidence,
    pub destination: Option<String>,
    pub existed: Option<bool>,
}

/// Result of one `put_batch_evidence` entry.
#[derive(Debug)]
pub enum EvidencePutOutcome {
    /// Published under `path` (numbered when the requested name was taken);
    /// `evidence` is `None` when the host could not read the published file
    /// back, so the caller asks for it as before.
    Published {
        path: String,
        evidence: Option<FileEvidence>,
    },
    Failed(io::Error),
}

/// Called once per request part before the host may publish any entry of
/// it: the index of the part's first entry and, per entry, whether its
/// requested path existed when the host accepted the part (`None` = unknown).
/// An error ends the part with nothing of it published.
pub type PreparePut<'a> = dyn FnMut(usize, &[Option<bool>]) -> io::Result<()> + 'a;

/// Batch limits of `backend` anywhere in its namespace: `batch_limits` of
/// its root, else what its peer accepts below that root (a Share's root is
/// the list of exports, where no file lies).
pub fn host_batch_limits<B: Backend + ?Sized>(backend: &B) -> Option<super::BatchLimits> {
    backend.batch_limits(&backend.root_display()).or_else(|| {
        backend
            .extensions()
            .and_then(|extensions| extensions.host_batch_limits())
    })
}

/// Whether `backend` takes a single new file as a packet of one
/// (`BackendExtensions::single_put`); `false` without extensions.
pub fn single_put<B: Backend + ?Sized>(backend: &B) -> bool {
    backend
        .extensions()
        .is_some_and(|extensions| extensions.single_put())
}

/// The job guard of `backend` (`BackendExtensions::transfer_scope`); the
/// engine drops it when the job ends. `None` without extensions.
pub fn transfer_scope<B: Backend + ?Sized>(backend: &B) -> Option<Box<dyn std::any::Any + Send>> {
    backend
        .extensions()
        .and_then(|extensions| extensions.transfer_scope())
}

/// Whether `put_batch_evidence`, `promote_copy_stage_evidence`,
/// `open_read_evidence`, `take_stage_evidence` and the evidence of
/// `BatchSink::evidence` are served for files below `path`.
pub fn transfer_evidence<B: Backend + ?Sized>(backend: &B, path: &str) -> bool {
    backend
        .extensions()
        .is_some_and(|extensions| extensions.transfer_evidence(path))
}

/// `put_batch` whose outcomes carry the published files' evidence and whose
/// `prepare` learns, before any publication, which destinations existed.
/// `Unsupported` (nothing read from `data`) where the backend has no such
/// extension.
pub fn put_batch_evidence<B: Backend + ?Sized>(
    backend: &B,
    entries: &[BatchPut],
    data: &mut dyn Read,
    prepare: &mut PreparePut<'_>,
) -> VfsResult<Vec<EvidencePutOutcome>> {
    match backend.extensions() {
        Some(extensions) => extensions.put_batch_evidence(entries, data, prepare),
        None => super::meta::unsupported("backend reports no transfer evidence"),
    }
}

/// `promote_copy_stage`, exactly once; `Some` carries the published file's
/// evidence when the host reported it.
pub fn promote_copy_stage_evidence<B: Backend + ?Sized>(
    backend: &B,
    staged: &str,
    destination: &str,
) -> VfsResult<Option<FileEvidence>> {
    match backend.extensions() {
        Some(extensions) => extensions.promote_copy_stage_evidence(staged, destination),
        None => backend
            .promote_copy_stage(staged, destination)
            .map(|()| None),
    }
}

/// The whole file `path` (one of several equal names by `id`) with the
/// evidence the host read when it opened it; `None` = not offered, the
/// caller opens it as before.
pub fn open_read_evidence<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    id: Option<&str>,
) -> VfsResult<Option<(Box<dyn Read + Send>, FileEvidence)>> {
    match backend.extensions() {
        Some(extensions) => extensions.open_read_evidence(path, id),
        None => Ok(None),
    }
}

/// The evidence the host reported when this backend's write of `stage`
/// was committed; taken once.
pub fn take_stage_evidence<B: Backend + ?Sized>(backend: &B, stage: &str) -> Option<StageEvidence> {
    backend
        .extensions()
        .and_then(|extensions| extensions.take_stage_evidence(stage))
}

/// Whether `create_dirs` is served for folders below `root`.
pub fn supports_create_dirs<B: Backend + ?Sized>(backend: &B, root: &str) -> bool {
    backend
        .extensions()
        .is_some_and(|extensions| extensions.supports_create_dirs(root))
}

/// Creates `paths` (parents before children) like `create_dir` each, in
/// one request: per folder `Ok(true)` = created by this call, `Ok(false)` =
/// it existed and is merged into. `Unsupported` without the extension.
pub fn create_dirs<B: Backend + ?Sized>(
    backend: &B,
    paths: &[String],
) -> VfsResult<Vec<io::Result<bool>>> {
    match backend.extensions() {
        Some(extensions) => extensions.create_dirs(paths),
        None => super::meta::unsupported("backend creates no folders in bulk"),
    }
}
