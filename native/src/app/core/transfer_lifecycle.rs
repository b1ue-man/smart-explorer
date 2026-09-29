//! Transfers that ended and target connections that opened: completion
//! notices (Spec B), the error list into the app's error log, the file
//! clipboard of "Für andere Programme bereitstellen", and one refresh of the
//! view that shows a changed folder.
use super::prelude::*;
use super::transfer_center::{ConnectOutcome, TransferPurpose};
use super::transfer_route::{parent_dir, paste_job, TransferSelection};
use super::transfer_rows::{
    completion_notice, external_issues_text, group_digits, issue_count, issues_text, transfer_title,
};
use super::*;
use crate::transfer::{path_within, ExternalSnapshot, JobItems, TransferJob, TransferKind};

impl App {
    /// Collect ended transfers and opened targets; runs every frame.
    pub(in crate::app) fn drain_transfers(&mut self) {
        self.drain_transfer_targets();
        let ended = self
            .transfer_center
            .sync_externals(crate::transfer::external_snapshots());
        for snapshot in ended {
            self.report_external(snapshot);
        }
        let ids = self.transfer_center.poll();
        let mut refresh = false;
        for id in ids {
            refresh |= self.report_finished(id);
        }
        self.transfer_center.trim_finished();
        if refresh && !self.root_path.is_empty() {
            self.rescan();
        }
    }

    /// Report one finished transfer; true when the active view shows a
    /// folder it changed.
    fn report_finished(&mut self, id: u64) -> bool {
        let Some(entry) = self.transfer_center.finished_entry(id) else {
            return false;
        };
        let notice = completion_notice(entry);
        let log = (issue_count(entry) > 0).then(|| {
            (
                format!(
                    "Übertragung {}",
                    transfer_title(&entry.progress, entry.job.as_deref())
                ),
                issues_text(entry),
            )
        });
        let provide = match &entry.purpose {
            TransferPurpose::Provide { temp, sequence } => Some((
                temp.clone(),
                *sequence,
                entry.canceled || entry.failure.is_some(),
                issue_count(entry) > 0,
            )),
            TransferPurpose::Copy => None,
        };
        let refresh = entry.job.as_deref().is_some_and(|job| self.view_shows(job));
        if let Some((context, detail)) = log {
            self.push_app_error(context, detail);
        }
        self.notice = Some((notice, Instant::now()));
        if let Some((temp, sequence, abandoned, with_errors)) = provide {
            self.publish_provided(&temp, sequence, abandoned, with_errors);
        }
        refresh
    }

    /// Another program's copy from our data ended with errors (for example
    /// a selection too large for Explorer's file list): reported like our own
    /// transfer's. A clean end stays quiet; its notes (listing progress) are
    /// no warnings.
    fn report_external(&mut self, snapshot: ExternalSnapshot) {
        if snapshot.errors == 0 {
            return;
        }
        let errors = format!("{} Fehler", group_digits(snapshot.errors));
        let detail = match &snapshot.note {
            Some(note) => format!("{note} ({errors})"),
            None => errors,
        };
        let listed = external_issues_text(&snapshot);
        let log_detail = if listed.is_empty() {
            detail.clone()
        } else {
            format!("{detail}\n{listed}")
        };
        self.push_app_error(format!("Übergabe {}", snapshot.label), log_detail);
        self.notice = Some((
            format!(
                "⚠ {}: {detail} — Details unter „Übertragungen“",
                snapshot.label
            ),
            Instant::now(),
        ));
    }

    /// Start the transfers whose target connection is open now; list the
    /// ones that could not reach their target.
    fn drain_transfer_targets(&mut self) {
        for outcome in self.transfer_center.poll_connecting() {
            match outcome {
                ConnectOutcome::Ready {
                    selection,
                    target,
                    target_dir,
                } => match paste_job(&selection, &target, &target_dir, CopyMode::Copy) {
                    Ok(job) => {
                        self.submit_job(job);
                    }
                    Err(error) => {
                        let label = target.describe(&target_dir);
                        self.record_unstarted(&selection, label, error);
                    }
                },
                ConnectOutcome::Failed {
                    selection,
                    target_label,
                    message,
                } => {
                    let message = format!("Ziel nicht erreichbar: {message}");
                    self.record_unstarted(&selection, target_label, message);
                }
            }
        }
    }

    /// A transfer that never started stays in the list with its reason.
    pub(in crate::app) fn record_unstarted(
        &mut self,
        selection: &TransferSelection,
        target_label: String,
        message: String,
    ) {
        let kind = if selection.source.is_local() {
            TransferKind::Upload
        } else {
            TransferKind::RemoteCopy
        };
        let source = selection.describe_source();
        self.push_app_error(
            format!("Übertragung {source} → {target_label}"),
            message.clone(),
        );
        self.transfer_center
            .record_failure(kind, source, target_label, message.clone());
        self.notice = Some((format!("⚠ {message}"), Instant::now()));
    }

    /// Whether the active view shows a folder `job` changed: the target (or a
    /// folder around it) or, for a move, a source folder.
    fn view_shows(&self, job: &TransferJob) -> bool {
        if self.root_path.is_empty() {
            return false;
        }
        let place = self.current_place();
        let view = self.root_path.as_str();
        let related = |dir: &str| path_within(dir, view, false) || path_within(view, dir, false);
        if place.endpoint.same_namespace(&job.target) && related(job.target_dir.as_str()) {
            return true;
        }
        if job.mode != CopyMode::Move || !place.endpoint.same_namespace(&job.source) {
            return false;
        }
        match &job.items {
            JobItems::Roots { paths, .. } => {
                paths.iter().any(|path| related(parent_dir(path).as_str()))
            }
            JobItems::Pairs(pairs) => pairs
                .iter()
                .any(|pair| related(parent_dir(&pair.source).as_str())),
        }
    }
}
