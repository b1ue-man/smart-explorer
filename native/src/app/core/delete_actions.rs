use super::prelude::*;
use super::*;
use crate::app::delete_worker::DeleteReporter;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeleteIntent {
    Default,
    Permanent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeleteRoute {
    LocalRecycle,
    LocalPermanent,
    BackendRecycle,
    BackendPermanent,
    RecoverableOnly,
    Unsupported,
}

impl App {
    pub(in crate::app) fn trash_selected(&mut self) {
        if self.selection.is_empty() || !self.delete_slot_available() {
            return;
        }
        let remote = self
            .remote
            .as_ref()
            .map(|remote| (remote.backend.delete_disposition(), remote.backend.clone()));
        match resolve_delete_route(
            remote.as_ref().map(|(kind, _)| *kind),
            DeleteIntent::Default,
        ) {
            DeleteRoute::LocalRecycle => self.start_local_recycle(),
            DeleteRoute::BackendRecycle => {
                if let Some((_, backend)) = &remote {
                    self.start_backend_delete(DeleteKind::Recycle, backend.clone());
                }
            }
            DeleteRoute::BackendPermanent => {
                let count = self.selection.len();
                if confirm_yes_no(
                    "Remote endgültig löschen",
                    &format!(
                        "Dieser Server hat keinen Papierkorb. {count} Eintrag/Einträge endgültig löschen?"
                    ),
                ) {
                    if let Some((_, backend)) = &remote {
                        self.start_backend_delete(DeleteKind::Permanent, backend.clone());
                    }
                }
            }
            DeleteRoute::Unsupported => {
                self.error_msg = Some("Diese Quelle ist schreibgeschützt.".to_string());
            }
            DeleteRoute::LocalPermanent | DeleteRoute::RecoverableOnly => {
                self.error_msg = Some("Ungültige Löschroute.".to_string());
            }
        }
    }

    pub(in crate::app) fn delete_permanent(&mut self) {
        if self.selection.is_empty() || !self.delete_slot_available() {
            return;
        }
        let remote = self
            .remote
            .as_ref()
            .map(|remote| (remote.backend.delete_disposition(), remote.backend.clone()));
        match resolve_delete_route(
            remote.as_ref().map(|(kind, _)| *kind),
            DeleteIntent::Permanent,
        ) {
            DeleteRoute::LocalPermanent => {
                if self.confirm_permanent_delete() {
                    self.start_local_permanent_delete();
                }
            }
            DeleteRoute::BackendPermanent => {
                if self.confirm_permanent_delete() {
                    if let Some((_, backend)) = &remote {
                        self.start_backend_delete(DeleteKind::Permanent, backend.clone());
                    }
                }
            }
            DeleteRoute::RecoverableOnly => {
                self.error_msg = Some(
                    "Diese Quelle unterstützt nur wiederherstellbares Löschen; endgültiges Löschen ist nicht verfügbar."
                        .to_string(),
                );
            }
            DeleteRoute::Unsupported => {
                self.error_msg = Some("Diese Quelle ist schreibgeschützt.".to_string());
            }
            DeleteRoute::LocalRecycle | DeleteRoute::BackendRecycle => {
                self.error_msg = Some("Ungültige Löschroute.".to_string());
            }
        }
    }

    fn confirm_permanent_delete(&self) -> bool {
        confirm_yes_no(
            "Endgültig löschen",
            &format!(
                "{} Eintrag/Einträge UNWIDERRUFLICH löschen (nicht in den Papierkorb)?",
                self.selection.len()
            ),
        )
    }

    fn delete_slot_available(&mut self) -> bool {
        if self.trash_rx.is_some() || self.trash_worker.is_some() {
            self.notice = Some((
                "Ein Löschvorgang läuft bereits.".to_string(),
                std::time::Instant::now(),
            ));
            false
        } else {
            true
        }
    }

    fn selected_delete_targets(&self) -> Vec<crate::vfs::DeleteTarget> {
        let targets = self
            .entries
            .iter()
            .filter(|entry| self.selection.contains(&entry.key()))
            .map(|entry| crate::vfs::DeleteTarget {
                path: entry.path.to_string(),
                id: entry.id.as_ref().map(ToString::to_string),
                is_dir: entry.is_dir,
                is_symlink: entry.is_symlink,
            })
            .collect();
        collapse_delete_targets(targets)
    }

    fn start_local_recycle(&mut self) {
        let targets = self.selected_delete_targets();
        if targets.is_empty() {
            return;
        }
        let attempted = targets.len();
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let mut initial =
            DeleteProgress::new(DeleteKind::Recycle, DeleteOrigin::Explorer, attempted);
        initial.phase = DeletePhase::Applying;
        let worker_progress = initial.clone();
        let spawn = std::thread::Builder::new()
            .name("recycle".into())
            .spawn(move || {
                let mut outcome =
                    DeleteOutcome::new(DeleteKind::Recycle, DeleteOrigin::Explorer, attempted);
                let mut reporter = DeleteReporter::new(tx, worker_cancel.clone(), worker_progress);
                for target in targets {
                    if worker_cancel.load(Ordering::Acquire)
                        || !reporter.begin_opaque_apply(&target.path)
                    {
                        outcome.canceled = true;
                        break;
                    }
                    outcome.entries_planned = outcome.entries_planned.saturating_add(1);
                    let native =
                        PathBuf::from(target.path.replace('/', std::path::MAIN_SEPARATOR_STR));
                    match trash::delete(&native) {
                        Ok(()) => {
                            outcome.entries_deleted = outcome.entries_deleted.saturating_add(1);
                            outcome.record_success(target.path);
                            reporter.finish_target(true, true);
                        }
                        Err(error) => {
                            outcome.partial_mutation = true;
                            outcome.record_error(target.path, error.to_string());
                            reporter.finish_target(false, false);
                        }
                    }
                }
                if worker_cancel.load(Ordering::Acquire) && outcome.processed < outcome.attempted {
                    outcome.canceled = true;
                }
                reporter.finish(outcome);
            });
        self.install_delete_worker(spawn, rx, cancel, initial, DeleteOrigin::Explorer);
    }

    fn start_local_permanent_delete(&mut self) {
        let targets = self.selected_delete_targets();
        if targets.is_empty() {
            return;
        }
        let backend: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
        self.start_permanent_delete(targets, backend, "delete-permanent");
    }

    fn start_backend_delete(&mut self, kind: DeleteKind, backend: crate::vfs::BackendHandle) {
        let targets = self.selected_delete_targets();
        if targets.is_empty() {
            return;
        }
        if kind == DeleteKind::Permanent {
            self.start_permanent_delete(targets, backend, "backend-delete");
            return;
        }
        let attempted = targets.len();
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let mut initial = DeleteProgress::new(kind, DeleteOrigin::Explorer, attempted);
        initial.phase = DeletePhase::Applying;
        let worker_progress = initial.clone();
        let spawn = std::thread::Builder::new()
            .name("backend-delete".into())
            .spawn(move || {
                let mut outcome = DeleteOutcome::new(kind, DeleteOrigin::Explorer, attempted);
                let mut reporter = DeleteReporter::new(tx, worker_cancel.clone(), worker_progress);
                for target in targets {
                    if worker_cancel.load(Ordering::Acquire)
                        || !reporter.begin_opaque_apply(&target.path)
                    {
                        outcome.canceled = true;
                        break;
                    }
                    outcome.entries_planned = outcome.entries_planned.saturating_add(1);
                    let result = if target.is_dir && !target.is_symlink {
                        backend.remove_dir(&target.path)
                    } else {
                        backend.remove_file_id(&target.path, target.id.as_deref())
                    };
                    match result {
                        Ok(()) => {
                            outcome.entries_deleted = outcome.entries_deleted.saturating_add(1);
                            outcome.record_success(target.path);
                            reporter.finish_target(true, true);
                        }
                        Err(error) => {
                            outcome.partial_mutation = true;
                            outcome.record_error(target.path, error.to_string());
                            reporter.finish_target(false, false);
                        }
                    }
                }
                if worker_cancel.load(Ordering::Acquire) && outcome.processed < outcome.attempted {
                    outcome.canceled = true;
                }
                reporter.finish(outcome);
            });
        self.install_delete_worker(spawn, rx, cancel, initial, DeleteOrigin::Explorer);
    }

    fn start_permanent_delete(
        &mut self,
        targets: Vec<crate::vfs::DeleteTarget>,
        backend: crate::vfs::BackendHandle,
        thread_name: &'static str,
    ) {
        let attempted = targets.len();
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let initial = DeleteProgress::new(DeleteKind::Permanent, DeleteOrigin::Explorer, attempted);
        let worker_progress = initial.clone();
        let spawn = std::thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                let mut outcome =
                    DeleteOutcome::new(DeleteKind::Permanent, DeleteOrigin::Explorer, attempted);
                let mut reporter = DeleteReporter::new(tx, worker_cancel.clone(), worker_progress);
                for target in targets {
                    if worker_cancel.load(Ordering::Acquire)
                        || !reporter.begin_target(&target.path, DeletePhase::Planning)
                    {
                        outcome.canceled = true;
                        break;
                    }
                    let result = crate::vfs::remove_entry_controlled(
                        &*backend,
                        &target,
                        &worker_cancel,
                        |progress| reporter.recursive_progress(progress),
                    );
                    match result {
                        Ok(report)
                            if report.status == crate::vfs::RecursiveDeleteStatus::Complete =>
                        {
                            outcome.entries_planned =
                                outcome.entries_planned.saturating_add(report.planned);
                            outcome.entries_deleted =
                                outcome.entries_deleted.saturating_add(report.removed);
                            outcome.record_success(target.path);
                            reporter.finish_target(true, false);
                        }
                        Ok(report) => {
                            outcome.entries_planned =
                                outcome.entries_planned.saturating_add(report.planned);
                            outcome.entries_deleted =
                                outcome.entries_deleted.saturating_add(report.removed);
                            outcome.processed = outcome.processed.saturating_add(1);
                            outcome.canceled = true;
                            outcome.partial_mutation |= report.removed > 0;
                            reporter.finish_target(false, false);
                            break;
                        }
                        Err(failure) => {
                            outcome.entries_planned =
                                outcome.entries_planned.saturating_add(failure.planned);
                            outcome.entries_deleted =
                                outcome.entries_deleted.saturating_add(failure.removed);
                            outcome.partial_mutation |= failure.removed > 0;
                            let detail = if failure.removed == 0 {
                                failure.error.to_string()
                            } else {
                                format!(
                                    "{}; {} von {} geplanten Einträgen bereits gelöscht",
                                    failure.error, failure.removed, failure.planned
                                )
                            };
                            outcome.record_error(target.path, detail);
                            reporter.finish_target(false, false);
                        }
                    }
                }
                if worker_cancel.load(Ordering::Acquire) && outcome.processed < outcome.attempted {
                    outcome.canceled = true;
                }
                reporter.finish(outcome);
            });
        self.install_delete_worker(spawn, rx, cancel, initial, DeleteOrigin::Explorer);
    }

    pub(in crate::app) fn install_delete_worker(
        &mut self,
        spawn: std::io::Result<std::thread::JoinHandle<()>>,
        rx: Receiver<DeleteMsg>,
        cancel: Arc<AtomicBool>,
        initial: DeleteProgress,
        origin: DeleteOrigin,
    ) {
        match spawn {
            Ok(worker) => {
                self.trash_rx = Some(rx);
                self.trash_worker = Some(worker);
                self.trash_cancel = Some(cancel);
                self.trash_progress = Some(initial);
                self.trash_origin = Some(origin);
            }
            Err(error) => {
                self.trash_rx = None;
                self.trash_worker = None;
                self.trash_cancel = None;
                self.trash_progress = None;
                self.trash_origin = None;
                self.error_msg = Some(format!("Löschvorgang konnte nicht starten: {error}"));
            }
        }
    }

    pub(in crate::app) fn cancel_delete_job(&mut self) {
        if let Some(cancel) = &self.trash_cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}

fn resolve_delete_route(
    remote: Option<crate::vfs::DeleteDisposition>,
    intent: DeleteIntent,
) -> DeleteRoute {
    match (remote, intent) {
        (None, DeleteIntent::Default) => DeleteRoute::LocalRecycle,
        (None, DeleteIntent::Permanent) => DeleteRoute::LocalPermanent,
        (Some(crate::vfs::DeleteDisposition::Recycle), DeleteIntent::Default) => {
            DeleteRoute::BackendRecycle
        }
        (Some(crate::vfs::DeleteDisposition::Permanent), _) => DeleteRoute::BackendPermanent,
        (Some(crate::vfs::DeleteDisposition::Recycle), DeleteIntent::Permanent) => {
            DeleteRoute::RecoverableOnly
        }
        (Some(crate::vfs::DeleteDisposition::Unsupported), _) => DeleteRoute::Unsupported,
    }
}

fn collapse_delete_targets(
    mut targets: Vec<crate::vfs::DeleteTarget>,
) -> Vec<crate::vfs::DeleteTarget> {
    targets.sort_by_key(|target| target.path.matches('/').count());
    let mut kept: Vec<crate::vfs::DeleteTarget> = Vec::new();
    for target in targets {
        if kept
            .iter()
            .any(|parent| parent.is_dir && is_path_below(&target.path, &parent.path))
        {
            continue;
        }
        kept.push(target);
    }
    kept
}

fn is_path_below(path: &str, parent: &str) -> bool {
    let parent = parent.trim_end_matches('/');
    path.strip_prefix(parent)
        .is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_targets_are_collapsed_without_prefix_confusion() {
        let target = |path: &str, is_dir| crate::vfs::DeleteTarget {
            path: path.into(),
            id: None,
            is_dir,
            is_symlink: false,
        };
        let collapsed = collapse_delete_targets(vec![
            target("/root/a/file", false),
            target("/root/ab/file", false),
            target("/root/a", true),
        ]);
        assert_eq!(collapsed.len(), 2);
        assert!(collapsed.iter().any(|entry| entry.path == "/root/a"));
        assert!(collapsed.iter().any(|entry| entry.path == "/root/ab/file"));
    }

    #[test]
    fn remote_permanent_intent_never_routes_local() {
        for disposition in [
            crate::vfs::DeleteDisposition::Recycle,
            crate::vfs::DeleteDisposition::Permanent,
            crate::vfs::DeleteDisposition::Unsupported,
        ] {
            assert_ne!(
                resolve_delete_route(Some(disposition), DeleteIntent::Permanent),
                DeleteRoute::LocalPermanent
            );
        }
    }
}
