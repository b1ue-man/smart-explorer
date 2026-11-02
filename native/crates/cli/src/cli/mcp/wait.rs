//! Waiting for a job inside one tool call: ends at the job's end, after the
//! call's `wait_seconds`, or when the client cancels the call. Progress
//! notifications go out while waiting when the client sent a progress token.
use super::jobs::Job;
use rmcp::model::ProgressNotificationParam;
use rmcp::service::RequestContext;
use rmcp::RoleServer;
use std::time::Duration;
use tokio::time::Instant;

/// Seconds a call waits without `wait_seconds`: half of Codex's documented
/// 60-second default tool timeout, leaving room for the answer
/// (docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1).
pub(super) const DEFAULT_WAIT_SECONDS: u64 = 30;
/// Interval of progress notifications while waiting. Claude Code resets its
/// 30-minute stdio idle timeout on progress
/// (docs/refs/claude-code-plugins-mcp-2026-10-12.md §4).
const HEARTBEAT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Waited {
    Ended,
    Running,
    ClientCancelled,
}

pub(super) fn wait_duration(seconds: Option<u64>) -> Duration {
    Duration::from_secs(seconds.unwrap_or(DEFAULT_WAIT_SECONDS))
}

pub(super) async fn wait_for(
    job: &Job,
    wait: Duration,
    context: &RequestContext<RoleServer>,
) -> Waited {
    let started = Instant::now();
    let deadline = started + wait;
    let token = context.meta.get_progress_token();
    let mut next_beat = started + HEARTBEAT;
    let mut versions = job.subscribe();
    loop {
        if job.end().is_some() {
            return Waited::Ended;
        }
        if Instant::now() >= deadline {
            return Waited::Running;
        }
        tokio::select! {
            _ = context.ct.cancelled() => return Waited::ClientCancelled,
            changed = versions.changed() => {
                if changed.is_err() {
                    return if job.end().is_some() { Waited::Ended } else { Waited::Running };
                }
            }
            _ = tokio::time::sleep_until(next_beat.min(deadline)) => {
                if Instant::now() >= next_beat {
                    next_beat += HEARTBEAT;
                    if let Some(token) = token.clone() {
                        let progress = started.elapsed().as_secs_f64();
                        let notice = ProgressNotificationParam::new(token, progress)
                            .with_message(progress_message(job));
                        let _ = context.peer.notify_progress(notice).await;
                    }
                }
            }
        }
    }
}

fn progress_message(job: &Job) -> String {
    let progress = job
        .control()
        .map(|control| control.progress())
        .filter(|value| !value.is_null());
    match progress {
        Some(progress) => format!("{} ({}): {progress}", job.summary, job.id),
        None => format!("{} ({}) is running", job.summary, job.id),
    }
}
