//! "Papierkorb" for a remote Find & Reclaim result (A34): selected duplicate
//! copies go into the trash of the device that stores them, which checks
//! every copy's size and content (SHA-256) again before it moves anything.
//! Only duplicate copies qualify, and a group never loses its last copy.
use super::prelude::*;
use super::*;
use crate::analytics::ReclaimReport;
use crate::app::delete_worker::DeleteReporter;
use crate::vfs::RecycleOutcome;
use std::sync::atomic::{AtomicBool, Ordering};

fn remote_trash_plan(
    report: &ReclaimReport,
    selected: &HashSet<String>,
) -> crate::analytics::RecyclePlan {
    crate::analytics::recycle_plan(&report.duplicate_groups, selected)
}

impl App {
    /// Moves the selected duplicate copies of a remote result into their
    /// device's trash (where it has one).
    pub(in crate::app) fn trash_reclaim_remote(&mut self, report: ReclaimReport) {
        let Some(StorageScanSource::Remote { backend, .. }) = self.reclaim_source.clone() else {
            return;
        };
        if !crate::vfs::supports_recycle(&*backend, &report.root).unwrap_or(false) {
            self.error_msg = Some(
                "Dieser Ort hat auf seinem Gerät keinen Papierkorb – die Ergebnisse werden nur angezeigt."
                    .to_string(),
            );
            return;
        }
        let plan = remote_trash_plan(&report, &self.reclaim_selected);
        if plan.targets.is_empty() {
            self.error_msg = Some(
                "Auf der Gegenstelle lassen sich nur Duplikatkopien verschieben, nie die letzte Kopie einer Gruppe."
                    .to_string(),
            );
            return;
        }
        let mut detail = format!(
            "{} Duplikatkopie(n) ({}) in den Papierkorb der Gegenstelle verschieben?\nDie Gegenstelle prüft vorher Größe und Inhalt jeder Kopie.",
            plan.targets.len(),
            format_bytes(plan.bytes)
        );
        if plan.kept > 0 {
            detail.push_str(&format!(
                "\n{} Kopie(n) bleiben, damit keine Gruppe ihre letzte Kopie verliert.",
                plan.kept
            ));
        }
        if plan.skipped > 0 {
            detail.push_str(&format!(
                "\n{} Auswahl(en) bleiben unverändert (keine eindeutig adressierbare SHA-256-Duplikatkopie).",
                plan.skipped
            ));
        }
        if !confirm_yes_no("In Papierkorb verschieben", &detail) {
            return;
        }
        let targets = plan.targets;
        let attempted = targets.len();
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let mut initial =
            DeleteProgress::new(DeleteKind::Recycle, DeleteOrigin::Reclaim, attempted);
        initial.phase = DeletePhase::Applying;
        let worker_progress = initial.clone();
        let spawn = std::thread::Builder::new()
            .name("reclaim-remote-trash".into())
            .spawn(move || {
                let mut outcome =
                    DeleteOutcome::new(DeleteKind::Recycle, DeleteOrigin::Reclaim, attempted);
                let mut reporter = DeleteReporter::new(tx, worker_cancel.clone(), worker_progress);
                for (path, expected) in targets {
                    if worker_cancel.load(Ordering::Acquire) || !reporter.begin_opaque_apply(&path)
                    {
                        outcome.canceled = true;
                        break;
                    }
                    outcome.entries_planned = outcome.entries_planned.saturating_add(1);
                    match crate::vfs::recycle(&*backend, &path, &expected) {
                        Ok(RecycleOutcome::Recycled) => {
                            outcome.entries_deleted = outcome.entries_deleted.saturating_add(1);
                            outcome.record_success(path);
                            reporter.finish_target(true, true);
                        }
                        Ok(RecycleOutcome::Changed) => {
                            outcome.record_error(
                                path,
                                "Inhalt hat sich geändert – nicht verschoben".to_string(),
                            );
                            reporter.finish_target(false, false);
                        }
                        Err(error) => {
                            outcome.partial_mutation = true;
                            outcome.record_error(path, error.to_string());
                            reporter.finish_target(false, false);
                        }
                    }
                }
                reporter.finish(outcome);
            });
        self.install_delete_worker(spawn, rx, cancel, initial, DeleteOrigin::Reclaim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::{
        ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimItem,
    };

    fn group(paths: &[&str]) -> DuplicateGroup {
        DuplicateGroup {
            hash: ContentHash {
                algorithm: HashAlgorithm::Sha256,
                hex: "ab".repeat(32),
            },
            evidence: DuplicateEvidence::LocalSha256,
            size: 10,
            reclaimable: 10,
            items: paths
                .iter()
                .map(|path| ReclaimItem::new(path.to_string(), path.to_string(), 10, 0, false))
                .collect(),
        }
    }

    #[test]
    fn review_task_remote_trash_never_takes_the_last_copy() {
        let report = ReclaimReport {
            is_remote: true,
            duplicate_groups: vec![group(&["/a", "/b"]), group(&["/c", "/d", "/e"])],
            ..ReclaimReport::default()
        };
        let selected: HashSet<String> = ["/a", "/b", "/d", "/large.iso"]
            .iter()
            .map(|path| path.to_string())
            .collect();
        let plan = remote_trash_plan(&report, &selected);
        let mut targets: Vec<&str> = plan.targets.iter().map(|(path, _)| path.as_str()).collect();
        targets.sort_unstable();
        assert_eq!(targets, ["/b", "/d"]);
        assert_eq!((plan.kept, plan.skipped, plan.bytes), (1, 1, 20));
        assert!(plan.targets.iter().all(|(_, expected)| expected.size == 10
            && expected.sha256.as_deref() == Some("ab".repeat(32).as_str())));
    }
}
