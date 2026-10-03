use super::prelude::*;
use super::*;

impl App {
    /// Compare a saved setup's two locations without changing anything (the
    /// "ls-diff" the user asked for). Resolves endpoints off-thread (local or
    /// remote) and runs `bisync::preview_with` with the job's own options/filters.
    pub(in crate::app) fn launch_preview(&mut self, job: &crate::syncjobs::SyncJob) {
        if self.preview_running || self.apply_one_rx.is_some() {
            return;
        }
        let job = job.clone();
        self.preview_title = format!("{}  ⇄  {}", job.source, job.target);
        self.preview_job_id = Some(job.id.clone());
        let now = now_secs_i64();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.preview_cancel = Some(cancel.clone());
        let worker_cancel = cancel.clone();
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun);
                let result = (|| -> Result<crate::bisync::Preview, String> {
                    let fresh = current_preview_job(&job.id)?;
                    if fresh.source != job.source || fresh.target != job.target {
                        return Err("Setup wurde umgestellt; Liste neu laden und erneut vergleichen.".into());
                    }
                    let job = fresh;
                    job.validate()
                        .map_err(|error| format!("Ungültiges Setup: {error}"))?;
                    let gs = job.checked_glob_set()?;
                    let (mn, mx, af, bf) = job.checked_filter_bounds(now)?;
                    let opts = job.checked_opts(true)?;
                    let (a, ra) =
                        crate::connect::resolve_endpoint(&job.source).map_err(|e| e.to_string())?;
                    let (b, rb) =
                        crate::connect::resolve_endpoint(&job.target).map_err(|e| e.to_string())?;
                    let fresh = current_preview_job(&job.id)?;
                    if fresh.source != job.source || fresh.target != job.target {
                        return Err("Setup wurde während der Verbindung umgestellt; bitte erneut vergleichen.".into());
                    }
                    let f = crate::bisync::WalkFilter {
                        include_hidden: job.include_hidden,
                        ignore: &gs,
                        min_size: mn,
                        max_size: mx,
                        after_mtime_ms: af,
                        before_mtime_ms: bf,
                    };
                    let a = crate::vfs::sync_backend(a); let b = crate::vfs::sync_backend(b);
                    Ok(crate::bisync::preview_with(&*a, &ra, &*b, &rb, opts, &worker_cancel, &f,
                        crate::bisync::RunSettings::for_job(&job.id)))
                })()
                .unwrap_or_else(|e| crate::bisync::Preview {
                    error: Some(e),
                    ..Default::default()
                });
                let _ = tx.send(result);
            });
        match spawn {
            Ok(worker) => {
                self.track_desktop_sync_worker(worker, cancel.clone());
                self.preview_rx = Some(rx);
                self.preview_running = true;
                self.preview = None;
                self.show_preview = true;
            }
            Err(error) => {
                let detail = format!("Vorschau-Thread konnte nicht starten: {error}");
                self.preview_rx = None;
                self.preview_running = false;
                self.preview_cancel = None;
                self.preview = Some(crate::bisync::Preview {
                    error: Some(detail.clone()),
                    ..Default::default()
                });
                self.show_preview = true;
                self.error_msg = Some(detail);
            }
        }
    }

    pub(in crate::app) fn drain_preview(&mut self) {
        match self.preview_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(preview)) => {
                self.preview = Some(preview);
                self.preview_running = false;
                self.preview_rx = None;
                self.preview_cancel = None;
                self.reload_sync_jobs("Sync-Setups nach Vergleich neu laden");
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                let detail = "Vorschau-Thread wurde ohne Ergebnis beendet.".to_string();
                self.preview_running = false;
                self.preview_rx = None;
                self.preview_cancel = None;
                self.preview = Some(crate::bisync::Preview {
                    error: Some(detail.clone()),
                    ..Default::default()
                });
                self.error_msg = Some(detail);
            }
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => {}
        }
    }

    /// Consume exactly the displayed plan; the engine owns guarded apply/state.
    pub(in crate::app) fn apply_one_action(&mut self, job_id: String, action: crate::bisync::Action) {
        if self.apply_one_rx.is_some() || self.bisync_running || self.sync_running
            || self.conflict_resolution.is_some() || self.merge.is_some() { return; }
        let Some(preview) = self.preview.take() else { return; };
        if preview.error.is_some() || preview.blocked.is_some() || !preview.actions.contains(&action) {
            self.preview = Some(preview); return;
        }
        let shared = Arc::new(std::sync::Mutex::new(Some(preview))); let worker_plan = shared.clone();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false)); let worker_cancel = cancel.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        match std::thread::Builder::new().name("sync-one-recorded".into()).spawn(move || {
            let Some(preview) = worker_plan.lock().unwrap_or_else(|e| e.into_inner()).take() else { return; };
            let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String,String> {
                let job = current_preview_job(&job_id)?; job.validate()?;
                let (a, ra) = crate::connect::resolve_endpoint(&job.source)?;
                let (b, rb) = crate::connect::resolve_endpoint(&job.target)?;
                let fresh = current_preview_job(&job_id)?;
                if fresh.source != job.source || fresh.target != job.target {
                    return Err("Setup wurde umgestellt; bitte erneut vergleichen.".into());
                }
                let a = crate::vfs::sync_backend(a); let b = crate::vfs::sync_backend(b);
                let stats = crate::bisync::apply_preview_action(&*a, &ra, &*b, &rb, &preview,
                    &action, job.checked_opts(false)?, &worker_cancel).map_err(|e| e.to_string())?;
                Ok(format!("Datei synchronisiert ({} →, {} ←, {} gelöscht)", stats.a_to_b, stats.b_to_a, stats.deleted))
            })).unwrap_or_else(|_| Err("Einzelaktion endete ohne verlässliches Ergebnis; bitte neu vergleichen.".into()));
            let _ = tx.send(super::sync_preview_types::PreviewApplyResult { preview, action, result });
        }) {
            Ok(worker) => { self.track_desktop_sync_worker(worker, cancel.clone()); self.apply_one_rx = Some(rx); self.bisync_running = true; self.bisync_cancel = Some(cancel); }
            Err(error) => {
                self.preview = shared.lock().unwrap_or_else(|e| e.into_inner()).take();
                self.error_msg = Some(format!("Einzelaktion konnte nicht starten: {error}"));
            }
        }
    }

    pub(in crate::app) fn drain_apply_one(&mut self) {
        match self.apply_one_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(mut returned)) => {
                self.apply_one_rx = None; self.bisync_running = false; self.bisync_cancel = None;
                finish_preview_action(Some(&mut returned.preview), &returned.action, returned.result.is_ok());
                self.preview = Some(returned.preview);
                match returned.result {
                    Ok(message) => self.notice = Some((message, std::time::Instant::now())),
                    Err(error) => self.error_msg = Some(format!("Einzelaktion: {error}")),
                }
                if !self.root_path.is_empty() { self.rescan(); }
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.apply_one_rx = None; self.bisync_running = false; self.bisync_cancel = None;
                self.error_msg = Some("Einzelaktion endete ohne Ergebnis; bitte neu vergleichen.".into());
            }
            _ => {}
        }
    }
}

fn current_preview_job(id: &str) -> Result<crate::syncjobs::SyncJob,String> {
    crate::syncjobs::load().map_err(|e| e.to_string())?.into_iter().find(|job| job.id == id)
        .ok_or_else(|| "Gespeicherter Auftrag fehlt; bitte neu vergleichen.".into())
}

fn finish_preview_action(
    preview: Option<&mut crate::bisync::Preview>,
    action: &crate::bisync::Action,
    succeeded: bool,
) {
    if let (true, Some(preview)) = (succeeded, preview) {
        preview.actions.retain(|candidate| candidate != action);
    }
}

#[cfg(test)]
mod tests {
    use super::finish_preview_action;
    use crate::bisync::{Action, Preview};

    #[test]
    fn apply_one_removes_action_only_after_success() {
        let action = Action::CopyAtoB("one.txt".to_string());
        let mut preview = Preview {
            actions: vec![action.clone()],
            ..Default::default()
        };
        finish_preview_action(Some(&mut preview), &action, false);
        assert_eq!(preview.actions, vec![action.clone()]);
        finish_preview_action(Some(&mut preview), &action, true);
        assert!(preview.actions.is_empty());
    }
}
