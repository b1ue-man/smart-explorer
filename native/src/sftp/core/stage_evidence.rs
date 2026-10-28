//! Transfer evidence of the SFTP backend (G3-02, 2026-10-28): what the
//! server already answered while a copy stage was written and published
//! replaces the receipts' own `LSTAT` requests (`vfs::take_stage_evidence`,
//! `vfs::promote_copy_stage_evidence`).
//!
//! - A copy stage's writer sends `FSTAT` of its handle and `LSTAT` of the
//!   destination right behind its last `WRITE` (no serial round trip of its
//!   own). The handle was created exclusively, so `FSTAT` describes this
//!   client's regular file after every `WRITE` (the server handles one
//!   handle's requests in order); the size must equal the bytes written.
//! - The publication sends `RENAME` and `LSTAT` of the destination together;
//!   only a regular file of the stage's size counts as evidence.
//!
//! Evidence is exactly what `stat` (`LSTAT`, `metadata::to_vfs`) answers:
//! SFTP has no object identity (`vfs::file_identity` is `None`). Anything
//! else (a missing answer, a link, another size) is no evidence, and the
//! engine asks as before.
use super::metadata::{basename, to_vfs};
use crate::vfs::{FileEvidence, StageEvidence};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{Attrs, FileAttributes, StatusCode};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Committed stages kept for the engine; a full map is emptied (the
/// engine then asks, as without evidence).
const MAX_WRITTEN: usize = 4096;
/// The engine's upload stages: `<destination>.se-upload-<16 hex>`
/// (`engine/publish.rs`, `copy_commit.rs`).
const UPLOAD_STAGE_MARKER: &str = ".se-upload-";

/// Evidence of committed stages until their publication or removal (also
/// kept by the SMB backend, smb/core/stage_evidence.rs).
#[derive(Clone, Default)]
pub(crate) struct WrittenStages(Arc<Mutex<HashMap<String, StageEvidence>>>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl WrittenStages {
    pub(crate) fn record(&self, evidence: StageEvidence) {
        let mut map = lock(&self.0);
        if map.len() >= MAX_WRITTEN && !map.contains_key(&evidence.stage.path) {
            map.clear();
        }
        map.insert(evidence.stage.path.clone(), evidence);
    }

    /// `vfs::take_stage_evidence`: the engine's copy; the stage's own
    /// evidence stays for its publication (`proven`).
    pub(crate) fn take(&self, stage: &str) -> Option<StageEvidence> {
        lock(&self.0).get(stage).cloned()
    }

    /// The committed stage as its writer saw it.
    pub(crate) fn proven(&self, stage: &str) -> Option<FileEvidence> {
        lock(&self.0)
            .get(stage)
            .map(|evidence| evidence.stage.clone())
    }

    /// The stage was published or removed: its evidence is void.
    pub(crate) fn forget(&self, stage: &str) {
        lock(&self.0).remove(stage);
    }
}

/// What a copy stage's writer reports at its commit.
pub(super) struct StageReport {
    pub(super) stage: String,
    pub(super) destination: Option<String>,
    pub(super) stages: WrittenStages,
}

impl StageReport {
    /// A report for an engine upload stage; `None` for any other name.
    pub(super) fn for_stage(stage: &str, stages: &WrittenStages) -> Option<Self> {
        let destination = stage_destination(stage)?;
        Some(Self {
            stage: stage.to_string(),
            destination: Some(destination),
            stages: stages.clone(),
        })
    }

    /// Keeps the stage's evidence: `stage` is the `FSTAT` answer of its
    /// handle, `destination` the `LSTAT` answer of the destination, `written`
    /// the bytes the writer had acknowledged.
    pub(super) fn record(
        &self,
        stage: &Attrs,
        destination: Option<&Result<Attrs, SftpError>>,
        written: u64,
    ) {
        let Some(evidence) = regular_evidence(&self.stage, &stage.attrs) else {
            return;
        };
        if evidence.size != written {
            return;
        }
        self.stages.record(StageEvidence {
            stage: evidence,
            destination: self.destination.clone(),
            existed: destination.and_then(presence),
        });
    }
}

/// The destination an engine upload stage is meant for.
pub(crate) fn stage_destination(stage: &str) -> Option<String> {
    let (destination, tail) = stage.rsplit_once(UPLOAD_STAGE_MARKER)?;
    let random = tail.len() == 16 && tail.bytes().all(|byte| byte.is_ascii_hexdigit());
    (random && !destination.is_empty() && !destination.ends_with('/'))
        .then(|| destination.to_string())
}

/// Evidence of `path` from attributes the server answered: only a regular
/// file whose size the server named.
pub(super) fn regular_evidence(path: &str, attrs: &FileAttributes) -> Option<FileEvidence> {
    attrs.size?;
    let meta = to_vfs(basename(path), attrs);
    if meta.is_dir || meta.is_symlink || meta.special {
        return None;
    }
    Some(FileEvidence {
        path: path.to_string(),
        object: None,
        id: meta.id,
        size: meta.size,
        mtime_ms: meta.mtime_ms,
    })
}

/// Whether an `LSTAT` answer proves something is at the path (`Some(true)`)
/// or nothing is (`Some(false)`); any other failure proves neither.
pub(super) fn presence(answer: &Result<Attrs, SftpError>) -> Option<bool> {
    match answer {
        Ok(_) => Some(true),
        Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
            Some(false)
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_sftp_stage_destination_only_for_engine_stages() {
        assert_eq!(
            stage_destination("/up/a b.txt.se-upload-0123456789abcdef").as_deref(),
            Some("/up/a b.txt")
        );
        assert_eq!(stage_destination("/up/a.txt.se-upload-0123"), None);
        assert_eq!(
            stage_destination("/up/a.txt.se-upload-0123456789abcdeg"),
            None
        );
        assert_eq!(stage_destination("/up/a.txt"), None);
        assert_eq!(stage_destination("/up/.se-upload-0123456789abcdef"), None);
    }

    #[test]
    fn speed_task_sftp_evidence_only_for_regular_files_with_size() {
        let mut attrs = FileAttributes::empty();
        attrs.size = Some(5);
        attrs.mtime = Some(1_700_000_000);
        attrs.permissions = Some(0o100644);
        let evidence = regular_evidence("/up/a", &attrs).expect("regular file");
        assert_eq!(evidence.size, 5);
        assert_eq!(evidence.mtime_ms, 1_700_000_000_000);
        assert_eq!(evidence.object, None);
        attrs.permissions = Some(0o120777);
        assert!(regular_evidence("/up/a", &attrs).is_none(), "a link");
        attrs.permissions = Some(0o100644);
        attrs.size = None;
        assert!(regular_evidence("/up/a", &attrs).is_none(), "no size named");
    }

    #[test]
    fn speed_task_sftp_written_stages_keep_evidence_until_forgotten() {
        let stages = WrittenStages::default();
        let evidence = FileEvidence {
            path: "/up/a.se-upload-0123456789abcdef".to_string(),
            object: None,
            id: None,
            size: 3,
            mtime_ms: 1_000,
        };
        stages.record(StageEvidence {
            stage: evidence.clone(),
            destination: Some("/up/a".to_string()),
            existed: Some(false),
        });
        assert_eq!(
            stages.take(&evidence.path).and_then(|known| known.existed),
            Some(false)
        );
        assert_eq!(stages.proven(&evidence.path), Some(evidence.clone()));
        stages.forget(&evidence.path);
        assert!(stages.proven(&evidence.path).is_none());
    }
}
