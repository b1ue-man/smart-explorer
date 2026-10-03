//! How a finished attempt changes the job state and when a problem is
//! notified (RV1, contract V4). Pure functions: the store applies them under
//! the per-job lock.

use super::job_state::{
    AttemptOutcome, AttemptReport, BlockKind, Blocked, FailureKind, JobSide, JobState, Notified,
    PendingKind, PendingTrigger, ProblemKind, ProblemNotice,
};
use super::types::SyncJob;

/// Waits after the first, second, … failure in a row. The series is short
/// first (a NAS that reboots, a network that comes back) and then slows down
/// to a few attempts a day, so an unreachable host or a full target is not
/// hammered; the last value repeats.
const RETRY_STEPS_SECS: [i64; 5] = [300, 900, 3_600, 3 * 3_600, 6 * 3_600];
/// A full or read-only target rarely clears within minutes.
const TARGET_FULL_STEPS_SECS: [i64; 2] = [3_600, 6 * 3_600];
/// The same problem is notified again at most once a day.
pub(super) const RENOTIFY_SECS: i64 = 86_400;

/// Applies one finished attempt (see contract V4, `record_attempt`).
pub(super) fn apply_attempt(state: &mut JobState, report: &AttemptReport) {
    state.last_attempt = Some(report.started);
    state.last_runner = Some(report.runner);
    state.last_cause = Some(report.cause);
    if let Some(result) = &report.result {
        state.last_result = Some(result.clone());
    }
    if state
        .running
        .as_ref()
        .is_some_and(|mark| mark.runner == report.runner && mark.started == report.started)
    {
        state.running = None;
    }
    match &report.outcome {
        AttemptOutcome::Success => {
            state.last_success = Some(report.finished);
            state.consecutive_failures = 0;
            state.last_error = None;
            state.blocked = None;
            state.retry_at = None;
            clear_covered_trigger(state, report.started);
        }
        AttemptOutcome::Failed(error) => {
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            state.last_error = Some(error.clone());
            state.retry_at = retry_at(error.kind, state.consecutive_failures, report.finished);
        }
        AttemptOutcome::Cancelled => {}
        AttemptOutcome::Blocked(block) => {
            let since = match &state.blocked {
                Some(previous)
                    if std::mem::discriminant(&previous.kind)
                        == std::mem::discriminant(&block.kind) =>
                {
                    previous.since.min(block.since)
                }
                _ => block.since,
            };
            state.blocked = Some(Blocked {
                since,
                confirmed: false,
                ..block.clone()
            });
            state.retry_at = None;
            clear_covered_trigger(state, report.started);
        }
    }
}

fn clear_covered_trigger(state: &mut JobState, started: i64) {
    if state
        .pending_trigger
        .as_ref()
        .is_some_and(|trigger| trigger.since <= started)
    {
        state.pending_trigger = None;
    }
}

/// When to try again after the `failures`-th failure in a row; `None` when
/// only the user can fix it.
pub(super) fn retry_at(kind: FailureKind, failures: u32, finished: i64) -> Option<i64> {
    if kind.needs_user() {
        return None;
    }
    let steps: &[i64] = if kind == FailureKind::TargetFull {
        &TARGET_FULL_STEPS_SECS
    } else {
        &RETRY_STEPS_SECS
    };
    let index = usize::try_from(failures.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(steps.len() - 1);
    Some(finished.saturating_add(steps[index]))
}

/// Marks the shown block as confirmed and asks for the run that may pass it.
/// `false` when the job is not blocked by exactly `kind`.
pub(super) fn confirm(state: &mut JobState, kind: &BlockKind, now: i64) -> bool {
    match &mut state.blocked {
        Some(block) if &block.kind == kind => block.confirmed = true,
        _ => return false,
    }
    let since = state
        .pending_trigger
        .as_ref()
        .map_or(now, |trigger| trigger.since.min(now));
    state.pending_trigger = Some(PendingTrigger {
        kind: PendingKind::Confirmed,
        since,
        volume: None,
    });
    true
}

/// Identity of the current problem for the notification throttle.
pub(super) fn problem_key(state: &JobState) -> Option<String> {
    let kind = state.problem()?;
    let detail = match kind {
        ProblemKind::Blocked => state
            .blocked
            .as_ref()
            .map(|block| block_label(&block.kind))
            .unwrap_or("other"),
        ProblemKind::NeedsAction | ProblemKind::FailureSeries => state
            .last_error
            .as_ref()
            .map(|error| failure_label(error.kind))
            .unwrap_or("other"),
    };
    Some(format!("{}:{detail}", problem_label(kind)))
}

/// A notice is due for a new or changed problem, and again once a day.
pub(super) fn notice_due(notified: Option<&Notified>, key: &str, now: i64) -> bool {
    match notified {
        Some(last) if last.key == key => now.saturating_sub(last.at) >= RENOTIFY_SECS,
        _ => true,
    }
}

pub(super) fn build_notice(job: &SyncJob, state: &JobState, kind: ProblemKind) -> ProblemNotice {
    let name = if job.name.trim().is_empty() {
        job.id.as_str()
    } else {
        job.name.trim()
    };
    let (title, text) = match kind {
        ProblemKind::Blocked => (
            format!("Sync „{name}“ ist angehalten"),
            state
                .blocked
                .as_ref()
                .map(block_text)
                .unwrap_or_else(|| "Der Lauf wurde zur Sicherheit angehalten.".into()),
        ),
        ProblemKind::NeedsAction => (
            format!("Sync „{name}“ braucht Ihre Hilfe"),
            state
                .last_error
                .as_ref()
                .map(|error| format!("{} {}", error.message, action_hint(error.kind)))
                .unwrap_or_default(),
        ),
        ProblemKind::FailureSeries => (
            format!("Sync „{name}“ schlägt wiederholt fehl"),
            format!(
                "{} Versuche in Folge ohne Erfolg. Zuletzt: {}",
                state.consecutive_failures,
                state
                    .last_error
                    .as_ref()
                    .map(|error| error.message.as_str())
                    .unwrap_or("unbekannter Fehler")
            ),
        ),
    };
    ProblemNotice {
        job_id: job.id.clone(),
        job_name: name.to_string(),
        kind,
        title,
        text,
    }
}

fn block_text(block: &Blocked) -> String {
    // The engine's own explanation says what happened and what to do.
    if !block.detail.trim().is_empty() {
        return block.detail.clone();
    }
    let explanation = match &block.kind {
        BlockKind::MassDelete {
            side,
            deletions,
            total,
        } => format!(
            "{deletions} von {total} Dateien in {} würden gelöscht.",
            side_label(*side)
        ),
        BlockKind::DeleteLimit { deletions, limit } => {
            format!("{deletions} Dateien würden gelöscht, erlaubt sind {limit}.")
        }
        BlockKind::SideEmpty { side, .. } => format!(
            "{} ist leer, war aber früher gefüllt – ist das Laufwerk angeschlossen?",
            side_label(*side)
        ),
        BlockKind::ReplicaMissing { side } => format!(
            "In {} fehlt die Sync-Markierung – anderes oder nicht eingehängtes Laufwerk?",
            side_label(*side)
        ),
        BlockKind::Other => "Der Lauf wurde zur Sicherheit angehalten.".to_string(),
    };
    format!("{explanation} Bitte prüfen und den Lauf bestätigen, wenn das so gewollt ist.")
}

fn side_label(side: JobSide) -> &'static str {
    match side {
        JobSide::A => "der Quelle",
        JobSide::B => "dem Ziel",
    }
}

fn action_hint(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Auth => "Bitte die Anmeldung der Verbindung prüfen.",
        FailureKind::Config => "Bitte die Einstellungen des Sync-Setups prüfen.",
        FailureKind::Access => "Bitte den Zugriff auf die Dateien erlauben.",
        _ => "",
    }
}

fn problem_label(kind: ProblemKind) -> &'static str {
    match kind {
        ProblemKind::Blocked => "blocked",
        ProblemKind::NeedsAction => "needs_action",
        ProblemKind::FailureSeries => "failures",
    }
}

fn block_label(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::MassDelete { .. } => "mass_delete",
        BlockKind::DeleteLimit { .. } => "delete_limit",
        BlockKind::SideEmpty { .. } => "side_empty",
        BlockKind::ReplicaMissing { .. } => "replica_missing",
        BlockKind::Other => "other",
    }
}

fn failure_label(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Config => "config",
        FailureKind::Unreachable => "unreachable",
        FailureKind::Auth => "auth",
        FailureKind::Access => "access",
        FailureKind::Hook => "hook",
        FailureKind::Run => "run",
        FailureKind::TargetFull => "target_full",
        FailureKind::Internal => "internal",
        FailureKind::Other => "other",
    }
}
