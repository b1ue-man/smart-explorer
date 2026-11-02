//! Folder deletion over WebDAV in one request: a DELETE on a collection acts
//! as if `Depth: infinity` were sent and removes the collection with all its
//! members (RFC 4918 §9.6.1), so neither a listing nor a request per member
//! is needed. A member the server cannot delete fails the request (207
//! Multi-Status, an error for `mutation`) after the server may already have
//! removed others; the failure says so and the DELETE is never repeated. When
//! the answer is lost, a Depth-0 PROPFIND decides: a collection that is gone
//! was deleted.
use super::core_impl::WebdavBackend;
use crate::vfs::{
    Backend, RecursiveDeleteFailure, RecursiveDeletePhase, RecursiveDeleteProgress,
    RecursiveDeleteReport, RecursiveDeleteStatus,
};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

/// The one entry the run plans: the collection itself.
const PLANNED: u64 = 1;

impl WebdavBackend {
    /// Delete the collection `path` with one DELETE after a Depth-0 PROPFIND
    /// proved it a collection; a file is refused (`InvalidInput`).
    pub(super) fn remove_collection(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        if cancel.load(Ordering::Acquire) {
            return Ok(report(RecursiveDeleteStatus::Canceled, 0));
        }
        let meta = self.stat(path).map_err(failure)?;
        if !meta.is_dir || meta.is_symlink {
            return Err(failure(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("WebDAV tree delete target is not a collection: {path}"),
            )));
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(report(RecursiveDeleteStatus::Canceled, 0));
        }
        applying(progress, path, 0);
        // The DELETE `remove_dir` sends: unpooled, without a Depth header.
        if let Err(error) = self.remove_dir(path) {
            // A lost answer (a large folder can outlast the 60-s inactivity
            // limit) leaves the outcome open: a collection that is gone was
            // deleted; one that is still there failed or is still going.
            if !matches!(self.stat(path), Err(gone) if gone.kind() == io::ErrorKind::NotFound) {
                return Err(failure(io::Error::new(
                    error.kind(),
                    format!(
                        "WebDAV DELETE of the folder {path} failed; the server may have removed part of it or still be removing it: {error}"
                    ),
                )));
            }
        }
        self.forget_stage_times_below(path);
        applying(progress, path, 1);
        Ok(report(RecursiveDeleteStatus::Complete, 1))
    }

    /// Stage times of entries that went with the collection.
    fn forget_stage_times_below(&self, path: &str) {
        let prefix = format!("{}/", path.trim_end_matches('/'));
        if let Ok(mut times) = self.stage_times.lock() {
            times.retain(|stage, _| !stage.starts_with(&prefix));
        }
    }
}

fn report(status: RecursiveDeleteStatus, removed: u64) -> RecursiveDeleteReport {
    RecursiveDeleteReport {
        status,
        planned: PLANNED,
        removed,
    }
}

fn failure(error: io::Error) -> RecursiveDeleteFailure {
    RecursiveDeleteFailure {
        error,
        planned: PLANNED,
        removed: 0,
    }
}

fn applying(progress: &mut dyn FnMut(RecursiveDeleteProgress), path: &str, removed: u64) {
    progress(RecursiveDeleteProgress {
        phase: RecursiveDeletePhase::Applying,
        planned: PLANNED,
        removed,
        current: path.to_string(),
    });
}
