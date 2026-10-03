//! Cancellable user commands, shared by daemon and manual runs.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use crate::syncjobs::{AttemptOutcome, SyncJob};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookPhase { Before, After, Cleanup }

pub fn run_job_hook(job: &SyncJob, phase: HookPhase, outcome: Option<&AttemptOutcome>,
    cancel: &AtomicBool) -> Result<(), String> {
    let command = match phase {
        HookPhase::Before => job.run_before.trim(),
        HookPhase::After => job.run_after.trim(),
        HookPhase::Cleanup => job.run_cleanup.trim(),
    };
    if command.is_empty() { return Ok(()); }
    let cleanup = phase == HookPhase::Cleanup;
    if !cleanup && cancel.load(Ordering::Acquire) { return Err("Befehl abgebrochen".into()); }
    let mut command = super::platform::shell_command(command);
    command.env("SE_JOB_ID", &job.id).env("SE_JOB_NAME", &job.name)
        .env("SE_SOURCE", &job.source).env("SE_TARGET", &job.target)
        .env("SE_HOOK_PHASE", match phase {
            HookPhase::Before => "before", HookPhase::After => "after", HookPhase::Cleanup => "cleanup",
        }).env("SE_RESULT", match outcome {
            Some(AttemptOutcome::Success) => "success", Some(AttemptOutcome::Failed(_)) => "failed",
            Some(AttemptOutcome::Cancelled) => "cancelled", Some(AttemptOutcome::Blocked(_)) => "blocked",
            None => "starting",
        });
    let mut child = super::platform::spawn_shell(command)
        .map_err(|error| format!("Befehl nicht startbar: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("Befehl fehlgeschlagen: {status}")),
            Ok(None) => {},
            Err(error) => { child.stop(); return Err(format!("Befehlstatus nicht lesbar: {error}")); }
        }
        if (!cleanup && cancel.load(Ordering::Acquire))
            || (cleanup && started.elapsed() >= super::job_supervisor::STOP_GRACE) {
            child.stop();
            return Err("Befehl abgebrochen".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
