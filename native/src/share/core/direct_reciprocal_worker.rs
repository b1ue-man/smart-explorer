use super::*;

pub(super) fn run_worker(
    node: Arc<ShareIrohNode>,
    shared: Arc<Shared>,
    completions: SyncSender<()>,
) {
    run_worker_with(shared, completions, |candidate| {
        node.repair_direct_reciprocal(
            &candidate.endpoint, &candidate.identity, candidate.key.local_generation,
        )
    }, || {
        let _ = node.ev.try_send(super::super::types::ShareEvent::Error(
            "Direct-Reparatur wurde unerwartet beendet; erneuter Versuch vorgemerkt".into(),
        ));
    });
}

fn run_worker_with(
    shared: Arc<Shared>,
    completions: SyncSender<()>,
    mut attempt: impl FnMut(&DirectRepairCandidate) -> DirectReciprocalTransportResult,
    report_panic: impl Fn(),
) {
    loop {
        let Some((key, epoch, candidate)) = take_due(&shared) else {
            return;
        };
        let result = run_attempt(|| attempt(&candidate), &report_panic);
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        if state.stopped {
            return;
        }
        let generation = state.generation;
        let Some(task) = state.tasks.get_mut(&key) else {
            continue;
        };
        if task.epoch != epoch || key.local_generation != generation {
            continue;
        }
        task.running = false;
        match result {
            DirectReciprocalTransportResult::Complete
            | DirectReciprocalTransportResult::AlreadyComplete => {
                task.blocked = true;
                task.due = None;
                task.candidate = None;
                // If this bounded channel is full, an older pending success is
                // already sufficient to trigger the same canonical reload.
                let _ = completions.try_send(());
            }
            DirectReciprocalTransportResult::Transient => {
                task.due = Some(Instant::now() + transient_delay(&key, task.transient_attempt));
                task.transient_attempt = task.transient_attempt.saturating_add(1);
                if task.candidate.is_none() {
                    task.candidate = Some(candidate);
                }
            }
            DirectReciprocalTransportResult::Unsupported => {
                task.due = Some(Instant::now() + unsupported_delay(&key, task.unsupported_attempt));
                task.unsupported_attempt = task.unsupported_attempt.saturating_add(1);
                if task.candidate.is_none() {
                    task.candidate = Some(candidate);
                }
            }
            DirectReciprocalTransportResult::PolicyDenied
            | DirectReciprocalTransportResult::Conflict => {
                task.blocked = true;
                task.due = None;
                task.candidate = None;
            }
        }
    }
}

fn take_due(shared: &Shared) -> Option<(DirectRepairKey, u64, DirectRepairCandidate)> {
    let mut state = shared.state.lock().ok()?;
    loop {
        if state.stopped {
            return None;
        }
        let now = Instant::now();
        let due_key = state
            .tasks
            .iter()
            .filter(|(_, task)| !task.running)
            .filter_map(|(key, task)| task.due.filter(|due| *due <= now).map(|due| (key, due)))
            .min_by_key(|(_, due)| *due)
            .map(|(key, _)| key.clone());
        if let Some(key) = due_key {
            let task = state.tasks.get_mut(&key)?;
            let candidate = task.candidate.take()?;
            task.running = true;
            task.due = None;
            return Some((key, task.epoch, candidate));
        }
        let wait = state
            .tasks
            .values()
            .filter(|task| !task.running)
            .filter_map(|task| task.due)
            .min()
            .map(|due| due.saturating_duration_since(now));
        state = match wait {
            Some(wait) => shared.wake.wait_timeout(state, wait).ok()?.0,
            None => shared.wake.wait(state).ok()?,
        };
    }
}

// Only the repair attempt is unwound here, outside the coordinator lock.
// Its permits/streams drop before the running marker is cleared below. A
// transport failure never becomes success and cannot replay a file mutation.
fn run_attempt(
    attempt: impl FnOnce() -> DirectReciprocalTransportResult,
    report_panic: impl FnOnce(),
) -> DirectReciprocalTransportResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(attempt)) {
        Ok(result) => result,
        Err(_) => {
            report_panic();
            DirectReciprocalTransportResult::Transient
        }
    }
}

#[cfg(test)]
#[path = "direct_reciprocal_worker_task_tests.rs"]
mod task_tests;
