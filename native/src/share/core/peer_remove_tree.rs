//! `remove_tree_v1` client: one stream asks the host to delete a folder and
//! follows its counters. A host without the feature never gets the request.
use crate::share::{
    backend::PeerBackend,
    fs_response::FsRemoveTreeMessage,
    peer_stream,
    wire::{FsRequest, FsResponse},
};
use crate::vfs::{
    RecursiveDeleteFailure, RecursiveDeletePhase, RecursiveDeleteProgress, RecursiveDeleteReport,
    RecursiveDeleteStatus,
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn remove(
    backend: &PeerBackend,
    path: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(RecursiveDeleteProgress),
) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
    let mut reported = Reported::default();
    if cancel.load(Ordering::Acquire) {
        return Ok(reported.canceled());
    }
    match peer_stream::features(backend, path) {
        Ok(features) if features.remove_tree_v1 => {}
        Ok(_) => {
            return Err(reported.failure(io::Error::new(
                io::ErrorKind::Unsupported,
                "Gegenstelle löscht keine Ordner in einem Auftrag",
            )))
        }
        Err(error) => return Err(reported.failure(error)),
    }
    let result = peer_stream::call(
        backend,
        FsRequest::RemoveTree { path: path.into() },
        cancel,
        |response| match response {
            FsResponse::RemoveTree { message } => reported.apply(path, message, &mut *progress),
            _ => Err(peer_stream::invalid(
                "Unerwartete Antwort auf das Ordnerlöschen",
            )),
        },
    );
    reported.finish(result, cancel)
}

/// The counters the host reported last; they never go backwards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Reported {
    planned: u64,
    removed: u64,
}

impl Reported {
    /// Apply one host message; `Some` is the final report of a complete run.
    fn apply(
        &mut self,
        path: &str,
        message: FsRemoveTreeMessage,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> io::Result<Option<RecursiveDeleteReport>> {
        match message {
            FsRemoveTreeMessage::Progress {
                applying,
                planned,
                removed,
            } => {
                self.advance(planned, removed)?;
                progress(RecursiveDeleteProgress {
                    phase: if applying {
                        RecursiveDeletePhase::Applying
                    } else {
                        RecursiveDeletePhase::Planning
                    },
                    planned,
                    removed,
                    current: path.to_string(),
                });
                Ok(None)
            }
            FsRemoveTreeMessage::Done { planned, removed } => {
                self.advance(planned, removed)?;
                Ok(Some(RecursiveDeleteReport {
                    status: RecursiveDeleteStatus::Complete,
                    planned,
                    removed,
                }))
            }
        }
    }

    /// The end of the stream: a cancel the caller set is `Canceled` with the
    /// last counters; every other error is a failure with them.
    fn finish(
        self,
        result: io::Result<RecursiveDeleteReport>,
        cancel: &AtomicBool,
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        match result {
            Ok(report) => Ok(report),
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted && cancel.load(Ordering::Acquire) =>
            {
                Ok(self.canceled())
            }
            Err(error) => Err(self.failure(error)),
        }
    }

    fn advance(&mut self, planned: u64, removed: u64) -> io::Result<()> {
        if planned < self.planned || removed < self.removed {
            return Err(peer_stream::invalid(
                "Löschzähler der Gegenstelle laufen rückwärts",
            ));
        }
        *self = Self { planned, removed };
        Ok(())
    }

    fn canceled(self) -> RecursiveDeleteReport {
        RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Canceled,
            planned: self.planned,
            removed: self.removed,
        }
    }

    fn failure(self, error: io::Error) -> RecursiveDeleteFailure {
        RecursiveDeleteFailure {
            error,
            planned: self.planned,
            removed: self.removed,
        }
    }
}

#[cfg(test)]
#[path = "peer_remove_tree_task_tests.rs"]
mod task_tests;
