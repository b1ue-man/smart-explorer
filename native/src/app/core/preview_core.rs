use super::prelude::*;
use super::*;

impl App {
    /// Compare a saved setup's two locations without changing anything (the
    /// "ls-diff" the user asked for). Resolves endpoints off-thread (local or
    /// remote) and runs `bisync::preview` with the job's own options/filters.
    pub(in crate::app) fn launch_preview(&mut self, job: &crate::syncjobs::SyncJob) {
        if self.preview_running {
            return;
        }
        let job = job.clone();
        self.preview_title = format!("{}  ⇄  {}", job.source, job.target);
        self.preview_job_id = Some(job.id.clone());
        let now = now_secs_i64();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.preview_cancel = Some(cancel.clone());
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                let result = (|| -> Result<crate::bisync::Preview, String> {
                    job.validate()
                        .map_err(|error| format!("Ungültiges Setup: {error}"))?;
                    let gs = job.checked_glob_set()?;
                    let (mn, mx, af, bf) = job.checked_filter_bounds(now)?;
                    let opts = job.checked_opts(true)?;
                    let (a, ra) =
                        crate::connect::resolve_endpoint(&job.source).map_err(|e| e.to_string())?;
                    let (b, rb) =
                        crate::connect::resolve_endpoint(&job.target).map_err(|e| e.to_string())?;
                    let f = crate::bisync::WalkFilter {
                        include_hidden: job.include_hidden,
                        ignore: &gs,
                        min_size: mn,
                        max_size: mx,
                        after_mtime_ms: af,
                        before_mtime_ms: bf,
                    };
                    Ok(crate::bisync::preview(
                        &*a, &ra, &*b, &rb, opts, &cancel, &f,
                    ))
                })()
                .unwrap_or_else(|e| crate::bisync::Preview {
                    error: Some(e),
                    ..Default::default()
                });
                let _ = tx.send(result);
            });
        match spawn {
            Ok(_) => {
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

    /// Apply a single planned action (one file) from the compare view, off-thread.
    pub(in crate::app) fn apply_one_action(
        &mut self,
        job_id: String,
        action: crate::bisync::Action,
    ) {
        if self.apply_one_rx.is_some() {
            return;
        }
        let job = match self.sync_jobs.iter().find(|j| j.id == job_id).cloned() {
            Some(j) => j,
            None => {
                self.error_msg = Some("Einzel-Sync: gespeicherter Auftrag fehlt.".to_string());
                return;
            }
        };
        let (tx, rx) = unbounded();
        let result_action = action.clone();
        let spawn = std::thread::Builder::new()
            .name("sync-one".into())
            .spawn(move || {
                let result = (|| -> Result<String, String> {
                    let (a, ra) =
                        crate::connect::resolve_endpoint(&job.source).map_err(|e| e.to_string())?;
                    let (b, rb) =
                        crate::connect::resolve_endpoint(&job.target).map_err(|e| e.to_string())?;
                    let vdir = crate::bisync::versions_dir(&crate::bisync::pair_id_for(
                        &*a, &ra, &*b, &rb,
                    ));
                    let cancel = std::sync::atomic::AtomicBool::new(false);
                    let mut errs = Vec::new();
                    let st = crate::bisync::apply(
                        &[action],
                        &*a,
                        &ra,
                        &*b,
                        &rb,
                        job.opts(false),
                        &vdir,
                        &mut errs,
                        &cancel,
                    );
                    if let Some((_, e)) = errs.first() {
                        return Err(e.clone());
                    }
                    Ok(format!(
                        "✓ 1 Datei synchronisiert ({}→ {}← {} gelöscht)",
                        st.a_to_b, st.b_to_a, st.deleted
                    ))
                })();
                let _ = tx.send((result_action, result));
            });
        match spawn {
            Ok(_) => self.apply_one_rx = Some(rx),
            Err(error) => {
                self.apply_one_rx = None;
                self.error_msg = Some(format!("Einzel-Sync-Thread konnte nicht starten: {error}"));
            }
        }
    }

    pub(in crate::app) fn drain_apply_one(&mut self) {
        match self.apply_one_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok((action, Ok(message)))) => {
                self.apply_one_rx = None;
                finish_preview_action(self.preview.as_mut(), &action, true);
                self.notice = Some((message, std::time::Instant::now()));
            }
            Some(Ok((action, Err(error)))) => {
                self.apply_one_rx = None;
                finish_preview_action(self.preview.as_mut(), &action, false);
                self.error_msg = Some(format!("Einzel-Sync: {error}"));
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.apply_one_rx = None;
                self.error_msg =
                    Some("Einzel-Sync-Thread wurde ohne Ergebnis beendet.".to_string());
            }
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => {}
        }
    }
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
