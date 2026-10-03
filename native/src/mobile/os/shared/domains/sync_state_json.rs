//! Explicit mobile job-state contract (times are milliseconds on the wire).
use crate::syncjobs::{JobState, ProblemKind};
use serde_json::{json, Value};

pub(super) fn attach(mut job: Value, state: &JobState, now: i64) -> Value {
    job["lastResult"] = state.last_result.as_ref().map(super::job_json::result_json).unwrap_or(Value::Null);
    job["state"] = json!({
        "lastAttemptMs": state.last_attempt.map(ms), "lastSuccessMs": state.last_success.map(ms),
        "lastRunner": state.last_runner, "lastCause": state.last_cause,
        "consecutiveFailures": state.consecutive_failures, "lastError": state.last_error,
        "retryAtMs": state.retry_at.map(ms), "loadError": state.load_error,
        "problem": state.problem().map(|kind| match kind {
            ProblemKind::Blocked => "blocked", ProblemKind::NeedsAction => "needs_action",
            ProblemKind::FailureSeries => "failure_series",
        }),
        "blocked": state.blocked.as_ref().map(|block| json!({
            "kind": block.kind, "detail": block.detail, "sinceMs": ms(block.since), "confirmed": block.confirmed,
        })),
        "running": state.running_now(now).map(|mark| json!({
            "runner": mark.runner, "cause": mark.cause, "startedMs": ms(mark.started),
            "aliveMs": ms(mark.alive), "stalledSinceMs": mark.stalled_since.map(ms),
        })),
        "pendingTrigger": state.pending_trigger.as_ref().map(|pending| json!({
            "kind": pending.kind, "sinceMs": ms(pending.since),
        })), "watch": state.watch,
        "lastVerifyMs": state.last_verify.map(ms), "verifyCursor": state.verify_cursor,
    });
    job
}
fn ms(secs: i64) -> i64 { secs.saturating_mul(1000) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn android_sync_state_keeps_attempt_success_and_live_runner_distinct() {
        let state = JobState { last_attempt: Some(200), last_success: Some(100),
            last_error: Some(crate::syncjobs::JobError { kind: crate::syncjobs::FailureKind::Access,
                message: "Dateizugriff fehlt".into() }),
            running: Some(crate::syncjobs::RunMark { runner: crate::syncjobs::Runner::Android,
                cause: crate::syncjobs::RunCause::Manual, started: 190, alive: 200, stalled_since: None }),
            ..JobState::default() };
        let row = attach(json!({"id":"job"}), &state, 200);
        assert_eq!(row["state"]["lastAttemptMs"], 200000);
        assert_eq!(row["state"]["lastSuccessMs"], 100000);
        assert_eq!(row["state"]["problem"], "needs_action");
        assert!(row["lastResult"].is_null());
        assert_eq!(row["state"]["running"]["startedMs"], 190000);
        assert!(attach(json!({"id":"job"}), &state, 10000)["state"]["running"].is_null());
    }
}
