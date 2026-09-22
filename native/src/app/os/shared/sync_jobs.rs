use super::prelude::*;
use super::*;

impl App {
    /// Resolve any remote endpoints off-thread, then run a saved sync setup.
    pub(in crate::app) fn run_job(&mut self, id: &str) {
        if self.bisync_running || self.sync_running || self.job_connect_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits ein Sync — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let job = match self.sync_jobs.iter().find(|j| j.id == id) {
            Some(j) => j.clone(),
            None => return,
        };
        if let Err(error) = checked_job_settings(&job) {
            self.error_msg = Some(format!("Ungültiges Sync-Setup: {error}"));
            return;
        }
        // Every path goes through the same resolver, including authenticated UNC.
        let (src, tgt) = (job.source.clone(), job.target.clone());
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("job-connect".into())
            .spawn(move || {
                let res = (|| {
                    let a = crate::connect::resolve_endpoint(&src)?;
                    let b = crate::connect::resolve_endpoint(&tgt)?;
                    Ok::<_, String>((a, b))
                })();
                let _ = tx.send(res);
            });
        match spawn {
            Ok(_) => {
                self.job_connect_rx = Some(rx);
                self.job_connect_pending = Some(job);
                self.notice = Some((
                    "Öffne Sync-Quelle und Ziel…".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.job_connect_rx = None;
                self.job_connect_pending = None;
                self.error_msg = Some(format!(
                    "Remote-Sync-Verbindung konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    /// Once a remote job's endpoints are open, launch the sync (UI thread).
    pub(in crate::app) fn drain_job_connect(&mut self) {
        let res = match self.job_connect_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.job_connect_rx = None;
                self.job_connect_pending = None;
                self.error_msg =
                    Some("Remote-Sync-Verbindung wurde ohne Ergebnis beendet.".to_string());
                return;
            }
        };
        self.job_connect_rx = None;
        let job = match self.job_connect_pending.take() {
            Some(j) => j,
            None => {
                self.error_msg = Some("Remote-Sync-Auftrag fehlt.".to_string());
                return;
            }
        };
        match res {
            Ok(((a, root_a), (b, root_b))) => {
                let (opts, bounds) = match checked_job_settings(&job) {
                    Ok(settings) => settings,
                    Err(error) => {
                        self.error_msg = Some(format!("Ungültiges Sync-Setup: {error}"));
                        return;
                    }
                };
                self.launch_bisync(
                    a,
                    root_a,
                    b,
                    root_b,
                    opts,
                    job.include_hidden,
                    job.ignore.clone(),
                    bounds,
                    Some(job.id.clone()),
                );
            }
            Err(e) => {
                self.error_msg = Some(format!("Remote-Sync: {}", e));
            }
        }
    }

}

type SyncFilterBounds = (u64, u64, i64, i64);
type CheckedJobSettings = (crate::bisync::BisyncOptions, SyncFilterBounds);

fn checked_job_settings(job: &crate::syncjobs::SyncJob) -> Result<CheckedJobSettings, String> {
    job.validate()?;
    Ok((
        job.checked_opts(false)?,
        job.checked_filter_bounds(now_secs_i64())?,
    ))
}
