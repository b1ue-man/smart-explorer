//! Client side of `evidence_v1` and `create_dirs_v1`: what the host reports
//! in its replies becomes `vfs::FileEvidence` bound to this connection
//! exactly as `stat` plus `vfs::file_identity` would describe the file, so
//! the transfer engine needs no request of its own per file. A host without
//! the features gets the requests of before.
use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

use crate::share::core::eio;
use crate::share::fs_error::into_io;
use crate::share::peer_extensions::file_identity;
use crate::share::wire::{
    discardable_stage, FsCreateDirs, FsDirOutcome, FsFileEvidence, FsRequest, FsResponse,
    FsWritten, CREATE_DIRS_MAX,
};
use crate::vfs::{FileEvidence, StageEvidence, VfsResult};

use super::PeerBackend;

/// Committed stages whose evidence the engine has not taken yet; beyond
/// this many the oldest are simply asked for again (no evidence kept).
const MAX_WRITTEN: usize = 4096;

/// Encoded folder paths of one `CreateDirs`, well below the host's 256 KiB
/// request frame.
const CREATE_DIRS_BYTES: usize = 128 * 1024;

/// The connection a reply's identities are bound to (`file_identity`).
pub(in crate::share) struct EvidenceBinding {
    binding: String,
    lease: Option<String>,
}

impl EvidenceBinding {
    pub(in crate::share) fn capture(backend: &PeerBackend) -> io::Result<Self> {
        Ok(Self {
            binding: file_identity::peer_binding(backend)?,
            lease: backend.mount_lease_token()?,
        })
    }

    /// Whether the connection and lease are still the ones captured.
    pub(in crate::share) fn still_current(&self, backend: &PeerBackend) -> bool {
        file_identity::peer_binding(backend).is_ok_and(|binding| binding == self.binding)
            && backend
                .mount_lease_token()
                .is_ok_and(|lease| lease == self.lease)
    }

    /// The evidence of `path` as this client's own requests would have
    /// described it. A path that looks like a private stage gets no object
    /// identity (as `file_identity::client` without a creator ticket);
    /// invalid evidence is dropped, so the caller asks as before.
    pub(in crate::share) fn file(
        &self,
        path: &str,
        evidence: FsFileEvidence,
    ) -> Option<FileEvidence> {
        let identity = evidence
            .identity
            .clone()
            .filter(|_| !file_identity::private_stage(path));
        self.bound(path, evidence, identity)
    }

    /// `file` of a stage this backend wrote itself (creator ticket held).
    fn owned_stage(&self, path: &str, evidence: FsFileEvidence) -> Option<FileEvidence> {
        let identity = evidence.identity.clone();
        self.bound(path, evidence, identity)
    }

    fn bound(
        &self,
        path: &str,
        evidence: FsFileEvidence,
        identity: Option<String>,
    ) -> Option<FileEvidence> {
        let object = file_identity::bind_client(&self.binding, identity).ok()?;
        Some(FileEvidence {
            path: path.to_string(),
            object,
            id: evidence.id,
            size: evidence.size,
            mtime_ms: evidence.mtime_ms,
        })
    }
}

/// Evidence of committed stages, until the engine takes it.
#[derive(Clone, Default)]
pub(in crate::share) struct WrittenStages(Arc<Mutex<HashMap<String, StageEvidence>>>);

impl WrittenStages {
    fn insert(&self, stage: &str, evidence: StageEvidence) {
        if let Ok(mut map) = self.0.lock() {
            if map.len() < MAX_WRITTEN || map.contains_key(stage) {
                map.insert(stage.to_string(), evidence);
            }
        }
    }

    pub(in crate::share) fn take(&self, stage: &str) -> Option<StageEvidence> {
        self.0.lock().ok().and_then(|mut map| map.remove(stage))
    }
}

/// What a stage write asked with `evidence` reports once committed.
pub(in crate::share) struct WrittenReport {
    pub(in crate::share) slot: Arc<Mutex<Option<FsWritten>>>,
    binding: EvidenceBinding,
    stage: String,
    destination: Option<String>,
    stages: WrittenStages,
}

impl WrittenReport {
    /// The file the stage is meant for, asked about at the commit.
    pub(in crate::share) fn destination(&self) -> Option<String> {
        self.destination.clone()
    }

    /// Keeps the committed stage's evidence for `take_stage_evidence`.
    pub(in crate::share) fn record(&self) {
        let Some(written) = self.slot.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        let Some(stage) = self.binding.owned_stage(&self.stage, written.evidence) else {
            return;
        };
        let existed = written.existed.filter(|_| self.destination.is_some());
        self.stages.insert(
            &self.stage,
            StageEvidence {
                stage,
                destination: self.destination.clone(),
                existed,
            },
        );
    }
}

/// The destination an engine upload stage (`<file>.se-upload-<16 hex>`)
/// is meant for.
fn stage_destination(stage: &str) -> Option<String> {
    let name = stage.rsplit('/').next().unwrap_or(stage);
    if !discardable_stage(name) {
        return None;
    }
    stage
        .rfind(".se-upload-")
        .and_then(|position| stage.get(..position))
        .map(str::to_string)
}

impl PeerBackend {
    /// The host answers with transfer evidence (`evidence_v1`).
    pub(in crate::share) fn evidence_offered(&self) -> bool {
        let caps = self.transfer_caps();
        caps.v1 && caps.evidence_v1
    }

    /// The host creates folders in bulk (`create_dirs_v1`).
    pub(in crate::share) fn create_dirs_offered(&self) -> bool {
        let caps = self.transfer_caps();
        caps.v1 && caps.create_dirs_v1
    }

    /// The host takes extra connections for transfers
    /// (`bulk_connections_v1`, Z-P6).
    pub(in crate::share) fn bulk_offered(&self) -> bool {
        let caps = self.transfer_caps();
        caps.v1 && caps.bulk_connections_v1
    }

    /// The host publishes a single file sent as a packet of one, with its
    /// evidence (`single_put_v1`, Z-P4).
    pub(in crate::share) fn single_put_offered(&self) -> bool {
        let caps = self.transfer_caps();
        caps.v1 && caps.single_put_v1 && caps.evidence_v1 && caps.batch_max_files > 0
    }

    /// The host gives each PutBatch entry its `mtime_ms` before publishing it
    /// (`batch_put_mtime_v1`).
    pub(in crate::share) fn batch_times_offered(&self) -> bool {
        let caps = self.transfer_caps();
        caps.v1 && caps.batch_put_mtime_v1 && caps.batch_max_files > 0
    }

    /// The WriteNew of an owned stage asks for its evidence and whether its
    /// destination exists; `None` for a host without `evidence_v1`.
    pub(in crate::share) fn written_report(&self, stage: &str) -> Option<WrittenReport> {
        if !self.evidence_offered() {
            return None;
        }
        let binding = EvidenceBinding::capture(self).ok()?;
        Some(WrittenReport {
            slot: Arc::new(Mutex::new(None)),
            binding,
            stage: stage.to_string(),
            destination: stage_destination(stage),
            stages: self.transfer.written.clone(),
        })
    }

    /// `vfs::take_stage_evidence`.
    pub(in crate::share) fn take_written(&self, stage: &str) -> Option<StageEvidence> {
        self.transfer.written.take(stage)
    }

    /// `vfs::promote_copy_stage_evidence`: the one publication of
    /// `promote_copy_stage`, answered with the published file.
    pub(in crate::share) fn promote_evidenced(
        &self,
        staged: &str,
        destination: &str,
    ) -> VfsResult<Option<FileEvidence>> {
        // Evidence of the stage is no use once it is published or gone.
        let _ = self.take_written(staged);
        if !self.evidence_offered() {
            return self
                .promote_reporting(staged, destination, true, false)
                .map(|_| None);
        }
        let binding = EvidenceBinding::capture(self)?;
        let evidence = self.promote_reporting(staged, destination, true, true)?;
        Ok(evidence
            .filter(|_| binding.still_current(self))
            .and_then(|evidence| binding.file(destination, evidence)))
    }

    /// `vfs::open_read_evidence`: the whole file by an `evidence_v1` ReadAt.
    pub(in crate::share) fn read_evidenced(
        &self,
        path: &str,
        id: Option<&str>,
    ) -> VfsResult<Option<(Box<dyn Read + Send>, FileEvidence)>> {
        if !self.evidence_offered() {
            return Ok(None);
        }
        let binding = EvidenceBinding::capture(self)?;
        let request = FsRequest::ReadAt {
            path: path.to_string(),
            id: id.map(str::to_string),
            offset: 0,
            len: 0,
            lz4: self.lz4_offered(),
            evidence: true,
        };
        let (reader, evidence) = self.open_read_stream(request, "peer read with evidence")?;
        let evidence = evidence
            .filter(|_| binding.still_current(self))
            .and_then(|evidence| binding.file(path, evidence));
        // Without evidence the stream is dropped and the caller reads as
        // before (one stream more, never a wrong record).
        Ok(evidence.map(|evidence| (reader, evidence)))
    }

    /// `vfs::create_dirs`: `CreateDirs` in portions of at most
    /// `CREATE_DIRS_MAX` folders and `CREATE_DIRS_BYTES` encoded paths.
    pub(in crate::share) fn create_dirs_bulk(
        &self,
        paths: &[String],
    ) -> VfsResult<Vec<io::Result<bool>>> {
        if !self.create_dirs_offered() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Die Gegenstelle legt keine Ordner gesammelt an",
            ));
        }
        let mut results = Vec::with_capacity(paths.len());
        let mut start = 0;
        while start < paths.len() {
            let mut end = start;
            let mut bytes = 0;
            while end < paths.len() && end - start < CREATE_DIRS_MAX {
                let next = paths.get(end).map_or(0, |path| path.len() + 4);
                if end > start && bytes + next > CREATE_DIRS_BYTES {
                    break;
                }
                bytes += next;
                end += 1;
            }
            let portion = paths.get(start..end).unwrap_or_default().to_vec();
            let count = portion.len();
            let request = FsRequest::CreateDirs(FsCreateDirs { paths: portion });
            let outcomes = match self.request(request)? {
                FsResponse::Dirs { outcomes } if outcomes.len() == count => outcomes,
                _ => return Err(eio("unerwartete Antwort auf create_dirs")),
            };
            results.extend(outcomes.into_iter().map(|outcome| match outcome {
                FsDirOutcome::Created => Ok(true),
                FsDirOutcome::Existed => Ok(false),
                FsDirOutcome::Failed { kind, msg } => Err(into_io(kind, msg)),
            }));
            start = end;
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::stage_destination;

    #[test]
    fn speed_task_upload_stage_names_their_destination() {
        assert_eq!(
            stage_destination("/B/dst/a.txt.se-upload-0123456789abcdef").as_deref(),
            Some("/B/dst/a.txt")
        );
        assert_eq!(stage_destination("/B/dst/a.txt.se-upload-xyz"), None);
        assert_eq!(stage_destination("/B/dst/a.txt"), None);
    }
}
