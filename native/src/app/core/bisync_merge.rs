use super::prelude::*;
use super::*;
use super::sync_merge_types::{MergeApplyResult, MergeDecision};
use std::sync::atomic::{AtomicBool, Ordering};

impl App {
    pub(in crate::app) fn start_merge(&mut self, rel: String) {
        if self.bisync_running || self.sync_running || self.conflict_resolution.is_some()
            || self.conflict_bulk.is_some() || self.merge.is_some() || self.merge_load_rx.is_some() || self.merge_apply_rx.is_some() { return; }
        let Some(conflict) = self.bisync_conflicts.iter().find(|c| c.rel == rel).cloned() else { return; };
        if conflict.duplicates.is_some() {
            self.error_msg = Some("Bitte zuerst eine konkrete Dateiversion auswählen.".into()); return;
        }
        let Some(context) = &self.bisync_ctx else { return; };
        let Some(key) = context.state.clone() else {
            self.error_msg = Some("Zusammenführen: gespeicherter Sync-Zustand fehlt.".into()); return;
        };
        let session = super::sync_merge_task::session(context, conflict, key);
        let cancel = Arc::new(AtomicBool::new(false)); let worker_cancel = cancel.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        match std::thread::Builder::new().name("merge-load".into()).spawn(move || {
            let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun);
            let _ = tx.send(super::sync_merge_task::load(session, &worker_cancel));
        }) {
            Ok(worker) => {
                self.track_desktop_sync_worker(worker, cancel.clone());
                self.merge = Some(MergeUi::loading(rel)); self.merge_load_rx = Some(rx);
                self.bisync_cancel = Some(cancel); self.bisync_running = true;
            }
            Err(error) => self.error_msg = Some(format!("Zusammenführen konnte nicht starten: {error}")),
        }
    }

    pub(in crate::app) fn drain_merge(&mut self) {
        match poll_merge_result(&self.merge_load_rx) {
            MergePoll::Ready(result) => {
                self.merge_load_rx = None; self.bisync_running = false; self.bisync_cancel = None;
                match result {
                    Ok(ui) => self.merge = Some(ui),
                    Err(error) => { self.merge = None; self.error_msg = Some(format!("Zusammenführen: {error}")); }
                }
            }
            MergePoll::Disconnected => {
                self.merge_load_rx = None; self.merge = None; self.bisync_running = false; self.bisync_cancel = None;
                self.error_msg = Some("Zusammenführen endete ohne Eingaben; Konflikt bleibt offen.".into());
            }
            MergePoll::Pending => {}
        }
        match poll_merge_result(&self.merge_apply_rx) {
            MergePoll::Ready(MergeApplyResult { mut ui, result }) => {
                self.merge_apply_rx = None; self.bisync_running = false; self.bisync_cancel = None;
                match result {
                    Ok(report) => {
                        if let Some(context) = self.bisync_ctx.as_mut() { context.baseline = report.baseline; }
                        self.conflict_baseline_dirty = false;
                        self.bisync_conflicts.retain(|conflict| conflict.rel != ui.rel);
                        self.merge = None;
                        if self.bisync_conflicts.is_empty() { self.finish_bisync_conflicts(); }
                        self.notice = Some((format!("„{}“ gespeichert; {} zusätzliche Dateien erhalten", ui.rel, report.preserved.len()), Instant::now()));
                    }
                    Err(failure) => {
                        let message = format!("{}; Quelle {}, Ziel {}. Originale und Entscheidung bleiben für „Erneut versuchen“ erhalten.", failure.error,
                            if failure.partial.confirmed_a { "bestätigt" } else { "offen" },
                            if failure.partial.confirmed_b { "bestätigt" } else { "offen" });
                        self.error_msg = Some(format!("Zusammenführen: {message}"));
                        ui.last_error = Some(message); self.merge = Some(ui);
                    }
                }
                if !self.root_path.is_empty() { self.rescan(); }
            }
            MergePoll::Disconnected => {
                self.merge_apply_rx = None; self.bisync_running = false; self.bisync_cancel = None;
                self.error_msg = Some("Zusammenführung endete ohne Ergebnis; gespeicherten Konflikt erneut öffnen.".into());
                self.merge = None;
            }
            MergePoll::Pending => {}
        }
    }

    pub(in crate::app) fn submit_merge(&mut self, mut ui: MergeUi, decision: Option<MergeDecision>) {
        if self.merge_apply_rx.is_some() || self.bisync_running || self.sync_running { self.merge = Some(ui); return; }
        if ui.retry.is_none() { ui.retry = decision; }
        let rel = ui.rel.clone();
        // Preserve the whole draft even if spawning the worker fails.
        let shared = Arc::new(std::sync::Mutex::new(Some(ui))); let worker_ui = shared.clone();
        let cancel = Arc::new(AtomicBool::new(false)); let worker_cancel = cancel.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        match std::thread::Builder::new().name("merge-recorded".into()).spawn(move || {
            let Some(mut ui) = worker_ui.lock().unwrap_or_else(|e| e.into_inner()).take() else { return; };
            let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::sync_merge_task::apply(&mut ui, &worker_cancel)))
                .unwrap_or_else(|_| Err(crate::bisync::MergeFailure { error:std::io::Error::other("Worker endete unerwartet; Originalentscheidung wiederholen"), partial:Default::default() }));
            let _ = tx.send(MergeApplyResult { ui, result });
        }) {
            Ok(worker) => {
                self.track_desktop_sync_worker(worker, cancel.clone());
                self.merge = Some(MergeUi::loading(rel)); self.merge_apply_rx = Some(rx);
                self.bisync_cancel = Some(cancel); self.bisync_running = true;
            }
            Err(error) => {
                self.merge = shared.lock().unwrap_or_else(|e| e.into_inner()).take();
                self.error_msg = Some(format!("Zusammenführen konnte nicht starten: {error}"));
            }
        }
    }

    pub(in crate::app) fn cancel_merge(&mut self) {
        if let Some(cancel) = &self.bisync_cancel { cancel.store(true, Ordering::Release); }
    }
}

enum MergePoll<T> { Pending, Ready(T), Disconnected }
fn poll_merge_result<T>(rx: &Option<Receiver<T>>) -> MergePoll<T> {
    match rx.as_ref().map(|rx| rx.try_recv()) {
        Some(Ok(value)) => MergePoll::Ready(value),
        Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => MergePoll::Disconnected,
        _ => MergePoll::Pending,
    }
}
