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
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                let result = (|| -> Result<crate::bisync::Preview, String> {
                    let (a, ra) =
                        crate::connect::resolve_endpoint(&job.source).map_err(|e| e.to_string())?;
                    let (b, rb) =
                        crate::connect::resolve_endpoint(&job.target).map_err(|e| e.to_string())?;
                    let gs = job.glob_set();
                    let (mn, mx, af, bf) = job.filter_bounds(now);
                    let f = crate::bisync::WalkFilter {
                        include_hidden: job.include_hidden,
                        ignore: &gs,
                        min_size: mn,
                        max_size: mx,
                        after_mtime_ms: af,
                        before_mtime_ms: bf,
                    };
                    Ok(crate::bisync::preview(
                        &*a,
                        &ra,
                        &*b,
                        &rb,
                        job.opts(true),
                        &cancel,
                        &f,
                    ))
                })()
                .unwrap_or_else(|e| crate::bisync::Preview {
                    error: Some(e),
                    ..Default::default()
                });
                let _ = tx.send(result);
            })
            .ok();
        self.preview_rx = Some(rx);
        self.preview_running = true;
        self.preview = None;
        self.show_preview = true;
    }

    pub(in crate::app) fn drain_preview(&mut self) {
        if let Some(p) = self.preview_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.preview = Some(p);
            self.preview_running = false;
            self.preview_rx = None;
            self.preview_cancel = None;
        }
    }

    /// Apply a single planned action (one file) from the compare view, off-thread.
    pub(in crate::app) fn apply_one_action(
        &mut self,
        job_id: String,
        action: crate::bisync::Action,
    ) {
        let job = match self.sync_jobs.iter().find(|j| j.id == job_id).cloned() {
            Some(j) => j,
            None => return,
        };
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("sync-one".into())
            .spawn(move || {
                let msg = (|| -> Result<String, String> {
                    let (a, ra) =
                        crate::connect::resolve_endpoint(&job.source).map_err(|e| e.to_string())?;
                    let (b, rb) =
                        crate::connect::resolve_endpoint(&job.target).map_err(|e| e.to_string())?;
                    let vdir = crate::bisync::versions_dir(&crate::bisync::pair_id(&ra, &rb));
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
                })()
                .unwrap_or_else(|e| format!("Fehler: {}", e));
                let _ = tx.send(msg);
            })
            .ok();
        self.apply_one_rx = Some(rx);
    }

    pub(in crate::app) fn drain_apply_one(&mut self) {
        if let Some(msg) = self.apply_one_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.apply_one_rx = None;
            self.notice = Some((msg, std::time::Instant::now()));
        }
    }
}
