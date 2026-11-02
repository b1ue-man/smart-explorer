//! The fake remote as an `evidence_v1` / `create_dirs_v1` Share host: it
//! reports what it read about the files it serves, publishes and closes, and
//! creates folders in bulk. The evidence is read from the folder below the
//! fake directly, so none of it counts as a request of the engine.
use super::Fake;
use crate::vfs::{
    BatchGet, BatchPut, BatchPutOutcome, BatchSink, EvidencePutOutcome, FileEvidence, PreparePut,
    StageEvidence, VfsResult,
};
use std::io::{self, Read};
use std::sync::atomic::Ordering;

impl Fake {
    /// What `stat` and `file_identity` of this fake answer for `path`.
    pub(super) fn evidence_of(&self, path: &str) -> Option<FileEvidence> {
        let meta = crate::vfs::Backend::stat(&self.inner, path).ok()?;
        if meta.is_dir || meta.is_symlink || meta.special {
            return None;
        }
        Some(FileEvidence {
            path: path.to_string(),
            object: crate::vfs::file_identity(&self.inner, path).ok()?,
            id: meta.id,
            size: meta.size,
            mtime_ms: meta.mtime_ms,
        })
    }

    fn exists(&self, path: &str) -> Option<bool> {
        match crate::vfs::Backend::stat(&self.inner, path) {
            Ok(_) => Some(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Some(false),
            Err(_) => None,
        }
    }

    pub(super) fn evidenced_put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
        prepare: &mut PreparePut<'_>,
    ) -> VfsResult<Vec<EvidencePutOutcome>> {
        if !self.evidence {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "fixture"));
        }
        let existed: Vec<Option<bool>> = entries.iter().map(|e| self.exists(&e.path)).collect();
        if let Err(error) = prepare(0, &existed) {
            // As the host: the part ends before its commit.
            let message = error.to_string();
            return Ok(entries
                .iter()
                .map(|_| {
                    let refused = crate::vfs::unpublished_error(error.kind(), message.clone());
                    EvidencePutOutcome::Failed(refused)
                })
                .collect());
        }
        Ok(self
            .fake_put_batch(entries, data)?
            .into_iter()
            .map(|outcome| match outcome {
                BatchPutOutcome::Published(path) => EvidencePutOutcome::Published {
                    evidence: self.evidence_of(&path),
                    path,
                },
                BatchPutOutcome::Failed(error) => EvidencePutOutcome::Failed(error),
            })
            .collect())
    }

    pub(super) fn evidenced_get_batch(
        &self,
        items: &[BatchGet],
        sink: &mut dyn BatchSink,
    ) -> VfsResult<()> {
        let mut announcing = Announcing {
            fake: self,
            items,
            sink,
        };
        self.fake_get_batch(items, &mut announcing)
    }

    pub(super) fn evidenced_read(
        &self,
        path: &str,
        _id: Option<&str>,
    ) -> VfsResult<Option<(Box<dyn Read + Send>, FileEvidence)>> {
        if !self.evidence {
            return Ok(None);
        }
        let Some(evidence) = self.evidence_of(path) else {
            return Ok(None);
        };
        let reader = crate::vfs::Backend::open_read(self, path)?;
        Ok(Some((reader, evidence)))
    }

    /// The stage as committed and whether the file it is meant for exists.
    pub(super) fn stage_evidence(&self, stage: &str) -> Option<StageEvidence> {
        if !self.evidence {
            return None;
        }
        let destination = stage
            .rfind(".se-upload-")
            .and_then(|position| stage.get(..position))
            .map(str::to_string);
        Some(StageEvidence {
            stage: self.evidence_of(stage)?,
            existed: destination.as_deref().and_then(|path| self.exists(path)),
            destination,
        })
    }

    pub(super) fn bulk_create_dirs(&self, paths: &[String]) -> VfsResult<Vec<io::Result<bool>>> {
        if !self.bulk_dirs {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "fixture"));
        }
        self.counters.bulk_dirs.fetch_add(1, Ordering::SeqCst);
        Ok(paths
            .iter()
            .map(
                |path| match crate::vfs::Backend::create_dir_new(&self.inner, path) {
                    Ok(()) => Ok(true),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let meta = crate::vfs::Backend::stat(&self.inner, path)?;
                        if meta.is_dir && !meta.is_symlink {
                            Ok(false)
                        } else {
                            Err(error)
                        }
                    }
                    Err(error) => Err(error),
                },
            )
            .collect())
    }
}

/// Announces each item's evidence before its bytes, as an `evidence_v1`
/// host's `EvidencedData` header does.
struct Announcing<'a> {
    fake: &'a Fake,
    items: &'a [BatchGet],
    sink: &'a mut dyn BatchSink,
}

impl BatchSink for Announcing<'_> {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        let evidence = self
            .items
            .get(index)
            .and_then(|item| self.fake.evidence_of(&item.path));
        if let Some(evidence) = evidence {
            self.sink.evidence(index, evidence)?;
        }
        self.sink.begin(index, size)
    }

    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        self.sink.data(index, bytes)
    }

    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        self.sink.end(index, result)
    }

    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()> {
        self.sink.failed(index, error)
    }
}
