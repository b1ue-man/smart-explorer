//! What the transfer list holds: running engine transfers (the lane, without
//! a fixed limit; transfers on one connection share it through its flow),
//! target folders on connections still being opened ("verbindet…"),
//! finished transfers with their issues and job (for "Fehlende übertragen"),
//! and transfers other programs perform from our data (Explorer pasting
//! virtual files). Pure state; the window and the App glue live elsewhere.
use super::transfer_route::{resume_job, TransferPlace, TransferSelection};
use crate::transfer::{
    ExternalSnapshot, LaunchTransfer, ResolvedRoot, TransferIssue, TransferJob, TransferKind,
    TransferLane, TransferProgress, TransferRequest,
};
use crate::vfs::BackendHandle;
use crossbeam_channel::{Receiver, TryRecvError};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

/// Delivers an opened target: its connection and the folder in its path form.
pub(in crate::app) type TargetRx = Receiver<Result<(BackendHandle, String), String>>;

/// Finished transfers and ended external hand-overs stay listed until removed
/// or the program ends; the list keeps the newest 30 of each (Spec B), so a
/// long session of pasting cannot grow it without bound.
pub(in crate::app) const FINISHED_KEPT: usize = 30;

/// What happens when a transfer ends besides being listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum TransferPurpose {
    /// Paste, drop and the copy/download dialogs.
    Copy,
    /// "Für andere Programme bereitstellen": the entries of the private
    /// download folder that `temp` lies in become a file clipboard while the
    /// OS clipboard still shows `sequence` (the user may have copied
    /// something else meanwhile).
    Provide {
        temp: PathBuf,
        sequence: Option<u32>,
    },
}

struct ActiveMeta {
    cancel: Arc<AtomicBool>,
    id: u64,
    purpose: TransferPurpose,
}

/// A target folder on a connection that is still being opened.
pub(in crate::app) struct ConnectingTarget {
    pub(in crate::app) id: u64,
    pub(in crate::app) selection: TransferSelection,
    /// Connection label for the opened place.
    pub(in crate::app) place_label: String,
    /// How the target is shown ("Verbindung: /Ordner").
    pub(in crate::app) target_label: String,
    pub(in crate::app) started: Instant,
    rx: TargetRx,
}

/// The result of opening a target connection.
pub(in crate::app) enum ConnectOutcome {
    Ready {
        selection: TransferSelection,
        target: TransferPlace,
        target_dir: String,
    },
    Failed {
        selection: TransferSelection,
        target_label: String,
        message: String,
    },
}

pub(in crate::app) struct FinishedEntry {
    pub(in crate::app) id: u64,
    pub(in crate::app) progress: TransferProgress,
    pub(in crate::app) canceled: bool,
    /// The transfer could not run at all or ended without a result.
    pub(in crate::app) failure: Option<String>,
    /// Display lines of a transfer that reported no structured issues.
    pub(in crate::app) errors: Vec<String>,
    pub(in crate::app) issues: Vec<TransferIssue>,
    pub(in crate::app) roots: Vec<ResolvedRoot>,
    pub(in crate::app) job: Option<Box<TransferJob>>,
    pub(in crate::app) purpose: TransferPurpose,
    /// The error list is unfolded in the window.
    pub(in crate::app) show_issues: bool,
}

impl FinishedEntry {
    /// Something is missing at the target: canceled, failed, errors, or
    /// fewer files done than found.
    pub(in crate::app) fn incomplete(&self) -> bool {
        self.canceled
            || self.failure.is_some()
            || self.progress.errors > 0
            || self.progress.files_done + self.progress.skipped < self.progress.files_total
    }

    /// "Fehlende übertragen" is possible for engine jobs that left something
    /// out. A provide download is not resumed: its private folder is removed
    /// when it ends incomplete, and a new request starts it afresh.
    pub(in crate::app) fn can_resume(&self) -> bool {
        self.job.is_some() && self.incomplete() && self.purpose == TransferPurpose::Copy
    }
}

/// An external hand-over, taken over at first sight so its notes and counts
/// stay visible after the providing side dropped its handle (K20).
pub(in crate::app) struct ExternalEntry {
    pub(in crate::app) snapshot: ExternalSnapshot,
    /// The providing side released its handle (the other program is done).
    pub(in crate::app) released: bool,
}

impl ExternalEntry {
    pub(in crate::app) fn ended(&self) -> bool {
        self.snapshot.finished || self.released
    }
}

#[derive(Default)]
pub(in crate::app) struct TransferCenter {
    pub(in crate::app) lane: TransferLane,
    /// One entry per running lane transfer, in lane order.
    meta: Vec<ActiveMeta>,
    pub(in crate::app) connecting: Vec<ConnectingTarget>,
    /// Newest first.
    pub(in crate::app) finished: VecDeque<FinishedEntry>,
    /// Newest first.
    pub(in crate::app) externals: Vec<ExternalEntry>,
    pub(in crate::app) window_open: bool,
    next_id: u64,
}

impl TransferCenter {
    fn allocate_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Start `job` at once through `launch` (the engine worker in the app).
    pub(in crate::app) fn submit(
        &mut self,
        job: TransferJob,
        purpose: TransferPurpose,
        launch: &mut LaunchTransfer<'_>,
    ) -> Result<u64, String> {
        self.lane
            .submit(TransferRequest::Job(Box::new(job)), launch)?;
        let cancel = self
            .lane
            .active
            .last()
            .map(|transfer| transfer.cancel.clone())
            .ok_or_else(|| "Übertragung wurde nicht aufgenommen".to_string())?;
        let id = self.allocate_id();
        self.meta.push(ActiveMeta {
            cancel,
            id,
            purpose,
        });
        Ok(id)
    }

    /// Row id of the running transfer at `index` of the lane.
    pub(in crate::app) fn active_id(&self, index: usize) -> Option<u64> {
        let transfer = self.lane.active.get(index)?;
        self.meta
            .iter()
            .find(|meta| Arc::ptr_eq(&meta.cancel, &transfer.cancel))
            .map(|meta| meta.id)
    }

    /// Take every transfer that ended into the finished list; returns their
    /// ids, newest last. Call `trim_finished` after reporting them.
    pub(in crate::app) fn poll(&mut self) -> Vec<u64> {
        let done = self.lane.poll();
        if done.is_empty() {
            return Vec::new();
        }
        // The lane removes ended transfers in lane order; the metadata of the
        // removed ones, in the same order, belongs to them one by one.
        let active = &self.lane.active;
        let (kept, mut removed): (Vec<ActiveMeta>, Vec<ActiveMeta>) =
            std::mem::take(&mut self.meta)
                .into_iter()
                .partition(|meta| {
                    active
                        .iter()
                        .any(|transfer| Arc::ptr_eq(&transfer.cancel, &meta.cancel))
                });
        self.meta = kept;
        removed.reverse();
        let mut ids = Vec::with_capacity(done.len());
        for finished in done {
            let (id, purpose) = match removed.pop() {
                Some(meta) => (meta.id, meta.purpose),
                None => (self.allocate_id(), TransferPurpose::Copy),
            };
            let kind = finished
                .job
                .as_ref()
                .map(|job| job.kind())
                .unwrap_or(TransferKind::Download);
            let entry = match finished.outcome {
                Some((progress, errors, worker_canceled)) => FinishedEntry {
                    id,
                    progress,
                    canceled: worker_canceled || finished.cancel_requested,
                    failure: None,
                    errors,
                    issues: finished.issues,
                    roots: finished.roots,
                    job: finished.job,
                    purpose,
                    show_issues: false,
                },
                None => {
                    let mut progress = TransferProgress::new(kind, kind.label(), 0, 0);
                    progress.done = true;
                    FinishedEntry {
                        id,
                        progress,
                        canceled: finished.cancel_requested,
                        failure: Some("Übertragungs-Thread wurde ohne Ergebnis beendet.".into()),
                        errors: Vec::new(),
                        issues: finished.issues,
                        roots: finished.roots,
                        job: finished.job,
                        purpose,
                        show_issues: false,
                    }
                }
            };
            self.finished.push_front(entry);
            ids.push(id);
        }
        ids
    }

    /// A transfer that could not start (for example an unreachable target):
    /// listed like a finished one with its message.
    pub(in crate::app) fn record_failure(
        &mut self,
        kind: TransferKind,
        source: String,
        target: String,
        message: String,
    ) -> u64 {
        let id = self.allocate_id();
        let mut progress = TransferProgress::new(kind, kind.label(), 0, 0);
        progress.done = true;
        progress.source = source;
        progress.target = target;
        self.finished.push_front(FinishedEntry {
            id,
            progress,
            canceled: false,
            failure: Some(message),
            errors: Vec::new(),
            issues: Vec::new(),
            roots: Vec::new(),
            job: None,
            purpose: TransferPurpose::Copy,
            show_issues: false,
        });
        id
    }

    /// Show a target that is being opened; `rx` delivers the connected
    /// backend and the folder in its path form.
    pub(in crate::app) fn connect_target(
        &mut self,
        selection: TransferSelection,
        place_label: String,
        target_label: String,
        rx: TargetRx,
    ) -> u64 {
        let id = self.allocate_id();
        self.connecting.push(ConnectingTarget {
            id,
            selection,
            place_label,
            target_label,
            started: Instant::now(),
            rx,
        });
        id
    }

    /// Targets whose connection attempt ended.
    pub(in crate::app) fn poll_connecting(&mut self) -> Vec<ConnectOutcome> {
        let mut outcomes = Vec::new();
        let mut index = 0;
        while index < self.connecting.len() {
            let result = match self.connecting[index].rx.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => {
                    index += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => {
                    Err("Verbindungsaufbau wurde ohne Ergebnis beendet".to_string())
                }
            };
            let target = self.connecting.remove(index);
            outcomes.push(match result {
                Ok((backend, target_dir)) => ConnectOutcome::Ready {
                    selection: target.selection,
                    target: TransferPlace::remote(backend, target.place_label),
                    target_dir,
                },
                Err(message) => ConnectOutcome::Failed {
                    selection: target.selection,
                    target_label: target.target_label,
                    message,
                },
            });
        }
        outcomes
    }

    /// Stop waiting for a target connection; its late result is dropped.
    pub(in crate::app) fn cancel_connecting(&mut self, id: u64) {
        self.connecting.retain(|target| target.id != id);
    }

    /// Take over the external hand-overs the providers currently track;
    /// returns the ones that ended since the last call, for their notice.
    pub(in crate::app) fn sync_externals(
        &mut self,
        live: Vec<ExternalSnapshot>,
    ) -> Vec<ExternalSnapshot> {
        let ended_before: Vec<u64> = self
            .externals
            .iter()
            .filter(|entry| entry.ended())
            .map(|entry| entry.snapshot.id)
            .collect();
        for entry in &mut self.externals {
            entry.released = !live.iter().any(|snapshot| snapshot.id == entry.snapshot.id);
        }
        for snapshot in live {
            match self
                .externals
                .iter_mut()
                .find(|entry| entry.snapshot.id == snapshot.id)
            {
                Some(entry) => entry.snapshot = snapshot,
                None => self.externals.insert(
                    0,
                    ExternalEntry {
                        snapshot,
                        released: false,
                    },
                ),
            }
        }
        self.externals
            .iter()
            .filter(|entry| entry.ended() && !ended_before.contains(&entry.snapshot.id))
            .map(|entry| entry.snapshot.clone())
            .collect()
    }

    /// Keep the newest `FINISHED_KEPT` finished transfers and ended hand-overs.
    pub(in crate::app) fn trim_finished(&mut self) {
        self.finished.truncate(FINISHED_KEPT);
        let mut ended = 0;
        self.externals.retain(|entry| {
            if !entry.ended() {
                return true;
            }
            ended += 1;
            ended <= FINISHED_KEPT
        });
    }

    pub(in crate::app) fn finished_entry(&self, id: u64) -> Option<&FinishedEntry> {
        self.finished.iter().find(|entry| entry.id == id)
    }

    pub(in crate::app) fn finished_entry_mut(&mut self, id: u64) -> Option<&mut FinishedEntry> {
        self.finished.iter_mut().find(|entry| entry.id == id)
    }

    /// Remove one finished transfer from the list.
    pub(in crate::app) fn remove_finished(&mut self, id: u64) {
        self.finished.retain(|entry| entry.id != id);
    }

    pub(in crate::app) fn remove_external(&mut self, id: u64) {
        self.externals.retain(|entry| entry.snapshot.id != id);
    }

    /// "Fertige entfernen": every finished transfer and ended hand-over.
    pub(in crate::app) fn clear_finished(&mut self) {
        self.finished.clear();
        self.externals.retain(|entry| !entry.ended());
    }

    /// The job for "Fehlende übertragen"; the old row gives way to the new run.
    pub(in crate::app) fn take_resume(&mut self, id: u64) -> Option<TransferJob> {
        let entry = self.finished_entry(id)?;
        if !entry.can_resume() {
            return None;
        }
        let job = resume_job(entry.job.as_deref()?, &entry.roots);
        self.remove_finished(id);
        Some(job)
    }

    pub(in crate::app) fn running_count(&self) -> usize {
        self.lane.active.len()
            + self.connecting.len()
            + self.externals.iter().filter(|entry| !entry.ended()).count()
    }

    pub(in crate::app) fn total_count(&self) -> usize {
        self.lane.active.len() + self.connecting.len() + self.finished.len() + self.externals.len()
    }

    /// No own transfer is running or connecting.
    pub(in crate::app) fn is_idle(&self) -> bool {
        self.lane.is_idle() && self.connecting.is_empty()
    }

    /// Program end: cancel everything; connection attempts are abandoned.
    pub(in crate::app) fn shutdown(&mut self) {
        self.lane.shutdown();
        self.meta.clear();
        self.connecting.clear();
    }
}

#[cfg(test)]
#[path = "transfer_center_tests.rs"]
mod tests;
