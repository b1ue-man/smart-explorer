//! Fresh saved-job runs, shared hooks/settings and durable attempt reporting.
use super::prelude::*;
use super::sync_run_state::{DesktopRun, JobConfirmation, RunMailbox};
use super::*;
use crate::syncjobs::{AttemptReport, RunCause, RunMark, Runner, SyncJob};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

impl App {
    pub(in crate::app) fn start_saved_desktop_run(
        &mut self,
        id: &str,
        confirmed: Option<JobConfirmation>,
    ) {
        if self.bisync_running
            || self.sync_running
            || self.job_connect_rx.is_some()
            || self.conflict_resolution.is_some()
            || self.merge_load_rx.is_some()
            || self.merge_apply_rx.is_some()
            || self.merge.is_some()
        {
            self.error_msg =
                Some("Bitte den laufenden Sync oder die Konfliktauflösung zuerst beenden.".into());
            return;
        }
        let id = id.to_owned();
        let display_id = id.clone();
        let mailbox = Arc::new(Mutex::new(RunMailbox::default()));
        let result_mailbox = mailbox.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let spawn = std::thread::Builder::new()
            .name("desktop-sync".into())
            .spawn(move || {
                let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun);
                let started = now_secs_i64();
                let cause = if confirmed.is_some() {
                    RunCause::Confirmed
                } else {
                    RunCause::Manual
                };
                let claimed = AtomicBool::new(false);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execute(
                        &id,
                        started,
                        confirmed,
                        &worker_cancel,
                        &result_mailbox,
                        &claimed,
                    )
                }));
                let mut out = match result {
                    Ok(out) => out,
                    Err(_) => failure("Sync", "Worker ohne verlässliches Ergebnis beendet".into()),
                };
                if let Some(key) = &out.state {
                    let pending = (|| -> std::io::Result<Vec<crate::bisync::Conflict>> {
                        let ctx = result_mailbox.lock().unwrap_or_else(|e| e.into_inner());
                        let Some(ctx) = &ctx.context else {
                            return Ok(Vec::new());
                        };
                        let relatives = {
                            let lock = crate::bisync::PairLock::acquire(&key.lock_id)?;
                            crate::bisync::pending_merge_relatives(&lock, key)?
                        };
                        let mut pending = Vec::new();
                        for rel in relatives {
                            if let Some(input) = crate::bisync::pending_merge_for_key(
                                &*ctx.a,
                                &ctx.root_a,
                                &*ctx.b,
                                &ctx.root_b,
                                key,
                                &rel,
                            )? {
                                if !pending
                                    .iter()
                                    .any(|c: &crate::bisync::Conflict| c.rel == input.conflict.rel)
                                {
                                    pending.push(input.conflict);
                                }
                            }
                        }
                        Ok(pending)
                    })();
                    match pending {
                        Ok(pending) => {
                            for conflict in &pending {
                                if !out.conflicts.iter().any(|c| c.rel == conflict.rel) {
                                    out.conflicts.push(conflict.clone());
                                }
                            }
                            result_mailbox
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .pending = pending;
                        }
                        Err(error) => {
                            out.stats.errors = out.stats.errors.saturating_add(1);
                            out.errors
                                .push(("Offene Zusammenführungen".into(), error.to_string()));
                        }
                    }
                }
                // Only our own live mark authorizes clearing/recording this attempt.
                if claimed.load(Ordering::Acquire) {
                    let finished = now_secs_i64();
                    let (mut outcome, result) = crate::syncjobs::classify_run(
                        &out,
                        worker_cancel.load(Ordering::Acquire),
                        finished,
                    );
                    if matches!(outcome, crate::syncjobs::AttemptOutcome::Failed(_)) {
                        if let Some(error) = result_mailbox
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .preparation_error
                            .clone()
                        {
                            outcome = crate::syncjobs::AttemptOutcome::Failed(error);
                        }
                    }
                    if let Err(error) = crate::syncjobs::record_attempt(
                        &id,
                        &AttemptReport {
                            runner: Runner::Desktop,
                            cause,
                            started,
                            finished,
                            outcome,
                            result: Some(result),
                        },
                    ) {
                        result_mailbox
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .persistence_errors
                            .push(format!("Laufzustand speichern: {error}"));
                    }
                }
                let _ = tx.send(out);
            });
        match spawn {
            Ok(worker) => {
                self.track_desktop_sync_worker(worker, cancel.clone());
                self.desktop_run = Some(DesktopRun { mailbox });
                self.bisync_ctx = None;
                self.running_job = Some(display_id);
                self.bisync_rx = Some(rx);
                self.bisync_cancel = Some(cancel);
                self.bisync_running = true;
                self.notice = Some((
                    "Sync wird mit den aktuellen Einstellungen vorbereitet…".into(),
                    Instant::now(),
                ));
            }
            Err(error) => self.error_msg = Some(format!("Sync konnte nicht starten: {error}")),
        }
    }
}

fn current_job(id: &str) -> Result<SyncJob, String> {
    crate::syncjobs::load()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|job| job.id == id)
        .ok_or_else(|| "Gespeicherter Auftrag wurde entfernt; nichts wird ausgeführt.".into())
}
fn failure(label: &str, error: String) -> crate::bisync::Outcome {
    let mut out = crate::bisync::Outcome::default();
    out.stats.errors = 1;
    out.errors.push((label.into(), error));
    out
}
fn execute(
    id: &str,
    started: i64,
    confirmed: Option<JobConfirmation>,
    cancel: &Arc<AtomicBool>,
    mailbox: &Arc<Mutex<RunMailbox>>,
    claimed: &AtomicBool,
) -> crate::bisync::Outcome {
    let result = (|| -> Result<crate::bisync::Outcome, String> {
        let job = current_job(id)?;
        let mut busy = false;
        let mut state_error = None;
        let mut accepted = false;
        let mut settings = crate::bisync::RunSettings::for_job(id);
        crate::syncjobs::update_job_state(id, |state| {
            if let Some(error) = &state.load_error {
                state_error = Some(error.clone());
                return;
            }
            if state.running_now(started).is_some() {
                busy = true;
                return;
            }
            state.running = Some(RunMark {
                runner: Runner::Desktop,
                cause: if confirmed.is_some() {
                    RunCause::Confirmed
                } else {
                    RunCause::Manual
                },
                started,
                alive: started,
                stalled_since: None,
            });
            accepted = true;
        })
        .map_err(|e| e.to_string())?;
        if let Some(error) = state_error {
            return Err(format!("Laufzustand nicht lesbar: {error}"));
        }
        if busy {
            return Ok(crate::bisync::Outcome {
                busy: true,
                ..Default::default()
            });
        }
        if accepted {
            claimed.store(true, Ordering::Release);
        }
        job.validate()
            .map_err(|e| format!("Ungültiges Sync-Setup: {e}"))?;
        if let Some(confirmation) = confirmed {
            if confirmation.source != job.source || confirmation.target != job.target {
                return Err(
                    "Ungültige Bestätigung: Setup wurde umgestellt; Quelle und Ziel erneut prüfen."
                        .into(),
                );
            }
            crate::syncjobs::confirm_block(id, &confirmation.kind).map_err(|e| e.to_string())?;
        }
        let state = crate::syncjobs::update_job_state(id, |state| {
            if let Some(block) = state.blocked.as_mut().filter(|block| block.confirmed) {
                if let Some(token) = crate::syncjobs::block_confirmation(&block.kind) {
                    settings.confirmed.push(token);
                    block.confirmed = false;
                }
            }
        })
        .map_err(|e| e.to_string())?;
        if let Some(block) = state.blocked.filter(|_| settings.confirmed.is_empty()) {
            return Ok(previous_block(&block));
        }
        let heartbeat = Heartbeat::start(id, started, cancel, mailbox)?;
        let before =
            crate::daemon::run_job_hook(&job, crate::daemon::HookPhase::Before, None, cancel)
                .map_err(|e| format!("Sync-Befehl: {e}"));
        let before_ok = before.is_ok();
        let work = before.and_then(|()| {
            if cancel.load(Ordering::Acquire) {
                return Err("Sync abgebrochen".into());
            }
            let (a, root_a) = crate::connect::resolve_endpoint(&job.source)?;
            let (b, root_b) = crate::connect::resolve_endpoint(&job.target)?;
            // Retargeting during a slow connection never applies an old queued setup.
            let fresh = current_job(id)?;
            if fresh.source != job.source || fresh.target != job.target {
                return Err(
                    "Setup wurde während der Vorbereitung umgestellt; bitte erneut starten.".into(),
                );
            }
            fresh.validate()?;
            let a = crate::vfs::sync_backend(a);
            let b = crate::vfs::sync_backend(b);
            let pair = crate::bisync::pair_id_for(&*a, &root_a, &*b, &root_b);
            mailbox.lock().unwrap_or_else(|e| e.into_inner()).context = Some(BisyncCtx {
                a: a.clone(),
                root_a: root_a.clone(),
                b: b.clone(),
                root_b: root_b.clone(),
                pair,
                state: None,
                job_id: Some(id.into()),
                baseline: Default::default(),
            });
            let glob = job.checked_glob_set()?;
            let bounds = job.checked_filter_bounds(now_secs_i64())?;
            let filter = crate::bisync::WalkFilter {
                include_hidden: job.include_hidden,
                ignore: &glob,
                min_size: bounds.0,
                max_size: bounds.1,
                after_mtime_ms: bounds.2,
                before_mtime_ms: bounds.3,
            };
            let mut request = crate::bisync::RunRequest::new(
                &*a,
                &root_a,
                &*b,
                &root_b,
                job.checked_opts(false)?,
                &filter,
                cancel,
            );
            request.settings = settings;
            Ok(crate::bisync::run_with(request))
        });
        let mut out = work.unwrap_or_else(|error| {
            remember_preparation_error(mailbox, &error);
            failure("Sync vorbereiten", error)
        });
        let (outcome, _) =
            crate::syncjobs::classify_run(&out, cancel.load(Ordering::Acquire), now_secs_i64());
        let phase = if matches!(outcome, crate::syncjobs::AttemptOutcome::Cancelled) {
            crate::daemon::HookPhase::Cleanup
        } else {
            crate::daemon::HookPhase::After
        };
        if before_ok || phase == crate::daemon::HookPhase::Cleanup {
            if let Err(error) = crate::daemon::run_job_hook(&job, phase, Some(&outcome), cancel) {
                if out.stats.errors == 0 {
                    remember_preparation_error(mailbox, &format!("Sync-Befehl: {error}"));
                }
                out.stats.errors = out.stats.errors.saturating_add(1);
                out.errors.push(("Sync-Befehl".into(), error));
            }
        }
        drop(heartbeat);
        Ok(out)
    })();
    match result {
        Ok(out) => out,
        Err(error) if claimed.load(Ordering::Acquire) => {
            remember_preparation_error(mailbox, &error);
            failure("Sync vorbereiten", error)
        }
        Err(error) => failure("Sync nicht gestartet", error),
    }
}

fn remember_preparation_error(mailbox: &Arc<Mutex<RunMailbox>>, message: &str) {
    let kind = if message.starts_with("Sync-Befehl:") {
        crate::syncjobs::FailureKind::Hook
    } else {
        crate::syncjobs::classify_failure(message)
    };
    mailbox
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .preparation_error = Some(crate::syncjobs::JobError {
        kind,
        message: message.into(),
    });
}

fn previous_block(block: &crate::syncjobs::Blocked) -> crate::bisync::Outcome {
    use crate::{
        bisync::{PairSide, RunBlock},
        syncjobs::{BlockKind, JobSide},
    };
    let side = |side| {
        if side == JobSide::A {
            PairSide::A
        } else {
            PairSide::B
        }
    };
    let blocked = match &block.kind {
        BlockKind::MassDelete {
            side: s,
            deletions,
            total,
        } => RunBlock::MassDelete {
            side: side(*s),
            deletes: *deletions,
            files: *total,
        },
        BlockKind::DeleteLimit { deletions, limit } => RunBlock::DeleteLimit {
            deletes: *deletions,
            limit: *limit,
        },
        BlockKind::SideEmpty { side: s, previous } => RunBlock::SideEmpty {
            side: side(*s),
            previous: *previous,
        },
        BlockKind::ReplicaMissing { side: s } => RunBlock::ReplicaMissing { side: side(*s) },
        BlockKind::Other => return failure("Sicherheitsstopp", block.detail.clone()),
    };
    crate::bisync::Outcome {
        blocked: Some(blocked),
        ..Default::default()
    }
}

struct Heartbeat {
    stop: crossbeam_channel::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Heartbeat {
    fn start(
        id: &str,
        started: i64,
        cancel: &Arc<AtomicBool>,
        mailbox: &Arc<Mutex<RunMailbox>>,
    ) -> Result<Self, String> {
        let (stop, rx) = crossbeam_channel::bounded(1);
        let id = id.to_owned();
        let cancel = cancel.clone();
        let mailbox = mailbox.clone();
        let worker = std::thread::Builder::new()
            .name("desktop-sync-state".into())
            .spawn(move || {
                while matches!(
                    rx.recv_timeout(std::time::Duration::from_secs(30)),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout)
                ) {
                    if let Err(error) = crate::syncjobs::update_job_state(&id, |state| {
                        if let Some(mark) = state.running.as_mut().filter(|mark| {
                            mark.runner == Runner::Desktop && mark.started == started
                        }) {
                            mark.alive = now_secs_i64();
                        }
                    }) {
                        mailbox
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .persistence_errors
                            .push(format!("Laufmeldung nicht speicherbar: {error}"));
                        cancel.store(true, Ordering::Release);
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for Heartbeat {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
