//! Admission and progress of one catch-up attempt; the book retains
//! completion ownership, cancellation and durable retry reporting.
use crate::syncjobs::SyncJob;

use super::{Admitted, CatchUpQueue, CatchUpSkip, EnqueueStatus, Phase, Run, ServiceReport};

pub(super) fn start_run(
    run: &mut Run,
    queue: &mut dyn CatchUpQueue,
    jobs: Option<&Result<Vec<SyncJob>, String>>,
    now: i64,
    report: &mut ServiceReport,
) {
    let jobs = match jobs {
        Some(Ok(jobs)) => jobs,
        Some(Err(error)) => {
            run.finish(
                format!("Sync-Jobs konnten nicht geladen werden: {error}"),
                report,
            );
            return;
        }
        // Requested after the caller decided not to load the list: the run
        // stays requested, and the next pass loads the list for it.
        None => return,
    };
    let selected: Vec<_> = jobs.iter().filter(|job| queue.eligible(job, now)).collect();
    for job in selected {
        let name = display_name(job);
        let admission = match queue.admit(job) {
            Ok(EnqueueStatus::Started | EnqueueStatus::Queued) => Ok(true),
            // The regular schedule already holds it (a cold wake enqueues the
            // due jobs first): wait for it, never cancel it.
            Ok(EnqueueStatus::AlreadyScheduled) => Ok(false),
            Ok(EnqueueStatus::RecentlyAttempted) => Err("kürzlich versucht".to_string()),
            Err(error) => Err(error),
        };
        match admission {
            Ok(owned) => run.admitted.push(Admitted {
                id: job.id.clone(),
                name,
                done: false,
                owned,
                outcome: None,
                ran: false,
            }),
            Err(reason) => run.skipped.push(CatchUpSkip {
                job_id: job.id.clone(),
                job_name: name,
                reason,
            }),
        }
    }
    run.phase = Phase::Running;
    report
        .started
        .push((run.id, run.admitted.len(), run.skipped.len()));
}

pub(super) fn update_progress(run: &mut Run, active: &[String], report: &mut ServiceReport) {
    let running = run
        .admitted
        .iter()
        .find(|job| !job.done && active.contains(&job.id));
    run.running_job = running.map(|job| job.name.clone());
    let undone = run.admitted.iter().filter(|job| !job.done).count();
    run.queued = undone
        - run
            .admitted
            .iter()
            .filter(|job| !job.done && active.contains(&job.id))
            .count();
    if undone == 0 {
        let message = run.message.clone().unwrap_or_else(|| {
            let ran = run.admitted.iter().filter(|job| job.ran).count();
            if ran == 0 && !run.admitted.is_empty() {
                "Kein Sync-Lauf abgeschlossen".into()
            } else {
                summary(ran, run.skipped.len())
            }
        });
        run.finish(message, report);
    }
}

fn summary(admitted: usize, skipped: usize) -> String {
    let done = match admitted {
        0 if skipped == 0 => return "Keine fälligen Jobs".into(),
        0 => "Kein Job gestartet".to_string(),
        1 => "1 Job ausgeführt".to_string(),
        count => format!("{count} Jobs ausgeführt"),
    };
    if skipped == 0 {
        done
    } else {
        format!("{done}, {skipped} übersprungen")
    }
}

fn display_name(job: &SyncJob) -> String {
    if job.name.trim().is_empty() {
        job.id.clone()
    } else {
        job.name.clone()
    }
}
