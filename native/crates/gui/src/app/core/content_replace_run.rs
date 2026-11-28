//! A running „In Dateien ersetzen“ and its „Rückgängig“ on background
//! threads (plan docs/plaene/2026-11-28-ersetzen-masken-shell, part C „Wo
//! ersetzt wird“, „Sicherung“): `vfs::content_replace` searches where the
//! files lie and writes each file through the backend of the place (stage +
//! `promote_staged`, the file checked unchanged first, the original backed
//! up before); a dry run only counts. This file is the one place of the
//! desktop app that speaks the core's replacement types.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, TryRecvError};

use crate::agent_proto::{HitRef, ReplaceEvent, ReplaceRequest, ReplaceSelection, ReplaceSummary};
use crate::vfs::ReplaceJournal;

use super::content_replace_results::{JobSelection, ReplaceJob};
use super::TextPlace;

/// What a run reports to the surface, in the surface's terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum ReplaceUpdate {
    /// A file with `replacements` places (changed, or to change in a dry run).
    File { rel: String, replacements: u64 },
    /// A file left unchanged, with the German reason.
    Skipped { rel: String, reason: String },
    /// Files searched so far.
    Progress { files: u64 },
}

/// The totals of a finished run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct RunTotals {
    pub(in crate::app) files_searched: u64,
    pub(in crate::app) files_changed: u64,
    pub(in crate::app) replacements: u64,
    pub(in crate::app) skipped: u64,
    /// The journal entry „Rückgängig“ restores (None: nothing was backed up).
    pub(in crate::app) undo_id: Option<String>,
    /// Ended early by „Abbrechen“; the files written before stay written
    /// and restorable.
    pub(in crate::app) canceled: bool,
}

fn totals(summary: ReplaceSummary) -> RunTotals {
    RunTotals {
        files_searched: summary.files_searched,
        files_changed: summary.files_changed,
        replacements: summary.replacements,
        skipped: summary.skipped,
        undo_id: summary.undo_id,
        canceled: summary.canceled,
    }
}

fn update(event: ReplaceEvent) -> Option<ReplaceUpdate> {
    Some(match event {
        ReplaceEvent::File {
            rel, replacements, ..
        } => ReplaceUpdate::File { rel, replacements },
        ReplaceEvent::Skipped { rel, reason } => ReplaceUpdate::Skipped {
            rel,
            reason: reason.message(),
        },
        ReplaceEvent::Progress { files_searched, .. } => ReplaceUpdate::Progress {
            files: files_searched,
        },
        // The places of a dry run are counted from its file events; the
        // surface shows no before/after text.
        ReplaceEvent::Preview { .. } => return None,
    })
}

/// The core's request of `job`.
pub(in crate::app) fn request(job: &ReplaceJob, dry_run: bool) -> ReplaceRequest {
    let selection = match &job.selection {
        JobSelection::All => ReplaceSelection::All,
        JobSelection::Hits(hits) => ReplaceSelection::Hits(
            hits.iter()
                .map(|hit| HitRef {
                    rel: hit.rel.clone(),
                    line: hit.line,
                    ranges: hit.ranges.clone(),
                    size: hit.size,
                    mtime_ms: hit.mtime_ms,
                })
                .collect(),
        ),
    };
    ReplaceRequest {
        query: job.query.clone(),
        replacement: job.replacement.clone(),
        selection,
        dry_run,
    }
}

/// Updates taken from one run per frame (the rest follows next frame).
const UPDATES_PER_FRAME: usize = 4096;

pub(in crate::app) struct ReplaceRun {
    /// The record the run fills (0 for the dry run of the confirmation).
    pub(in crate::app) id: u64,
    updates: Receiver<ReplaceUpdate>,
    done: Receiver<Result<RunTotals, String>>,
    cancel: Arc<AtomicBool>,
    finished: Option<Result<RunTotals, String>>,
}

impl ReplaceRun {
    /// Starts `job` (a dry run only counts); `Err` when the thread cannot start.
    pub(in crate::app) fn start(id: u64, job: &ReplaceJob, dry_run: bool) -> Result<Self, String> {
        let (update_tx, updates) = crossbeam_channel::unbounded();
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let backend = job.place.backend_for(&job.root);
        let root = job.root.clone();
        let location = job.place.location(&job.root);
        let request = request(job, dry_run);
        std::thread::Builder::new()
            .name("content-replace".into())
            .spawn(move || {
                let emit = |event: ReplaceEvent| match update(event) {
                    Some(update) => update_tx.send(update).is_ok(),
                    None => true,
                };
                let result = crate::vfs::content_replace(
                    backend.as_ref(),
                    &root,
                    &request,
                    &location,
                    &emit,
                    flag.as_ref(),
                )
                .map(totals)
                .map_err(|error| error.to_string());
                let _ = done_tx.send(result);
            })
            .map_err(|error| format!("Das Ersetzen konnte nicht starten: {error}"))?;
        Ok(Self {
            id,
            updates,
            done,
            cancel,
            finished: None,
        })
    }

    pub(in crate::app) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(in crate::app) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Waiting updates, and the end once every update was taken.
    pub(in crate::app) fn poll(
        &mut self,
    ) -> (Vec<ReplaceUpdate>, Option<Result<RunTotals, String>>) {
        if self.finished.is_none() {
            match self.done.try_recv() {
                Ok(result) => self.finished = Some(result),
                Err(TryRecvError::Disconnected) => {
                    self.finished = Some(Err("Das Ersetzen wurde unerwartet beendet.".to_string()))
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let updates: Vec<ReplaceUpdate> = self.updates.try_iter().take(UPDATES_PER_FRAME).collect();
        if !self.updates.is_empty() {
            return (updates, None);
        }
        (updates, self.finished.take())
    }
}

impl Drop for ReplaceRun {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// „Rückgängig“ of one record: the journal restores the backups of its run.
pub(in crate::app) struct UndoRun {
    pub(in crate::app) search: u64,
    /// (relative path, German reason) of the files left as they are.
    done: Receiver<Result<Vec<(String, String)>, String>>,
}

impl UndoRun {
    /// Restores the run `undo_id` below `root` of `place` (the backend of
    /// the run; another location names a place this window is not
    /// connected to).
    pub(in crate::app) fn start(
        search: u64,
        place: &TextPlace,
        root: &str,
        undo_id: String,
    ) -> Result<Self, String> {
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let backend = place.backend_for(root);
        let location = place.location(root);
        std::thread::Builder::new()
            .name("content-replace-undo".into())
            .spawn(move || {
                let resolve = |wanted: &str| -> crate::vfs::VfsResult<crate::vfs::BackendHandle> {
                    if wanted == location {
                        Ok(backend.clone())
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("Das Ziel {wanted} ist in diesem Fenster nicht verbunden."),
                        ))
                    }
                };
                let result = ReplaceJournal::open_default()
                    .and_then(|journal| journal.undo(&undo_id, &resolve))
                    .map(|summary| summary.skipped)
                    .map_err(|error| error.to_string());
                let _ = done_tx.send(result);
            })
            .map_err(|error| format!("Rückgängig konnte nicht starten: {error}"))?;
        Ok(Self { search, done })
    }

    /// The end of the restore, once it is there: the files it left (changed
    /// since the run).
    pub(in crate::app) fn poll(&self) -> Option<Result<Vec<(String, String)>, String>> {
        match self.done.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Disconnected) => {
                Some(Err("Rückgängig wurde unerwartet beendet.".to_string()))
            }
            Err(TryRecvError::Empty) => None,
        }
    }
}
