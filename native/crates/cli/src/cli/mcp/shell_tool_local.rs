//! A command of the `shell` tool on this device, as a job: started with se's
//! rights or elevated (`engine::elevate`; the system dialog is answered on
//! the job's worker, `{"state":"waiting_for_elevation"}` meanwhile), its
//! output in the job's `stdout`/`stderr` (raw streams, which the job masks
//! with the active mask lists), input through `job_input`, cancel and
//! timeout end its whole process tree. After the end the mask store is
//! checked (`shell_tool_guard.rs`).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use crate::elevate::{ElevateError, ElevatedEvent, ElevatedExit, ElevatedInput, ElevatedSpec};
use crate::masks::IntegrityGuard;

/// One command on this device.
pub(super) struct LocalRun {
    pub(super) spec: ElevatedSpec,
    /// Result name of the shell (`bash`, `pwsh 7`, `cmd`, `none`).
    pub(super) shell: String,
    pub(super) elevated: bool,
    pub(super) stdin: Option<String>,
    pub(super) keep_stdin_open: bool,
    pub(super) timeout_seconds: Option<u64>,
    pub(super) summary: String,
}

#[derive(Default)]
struct LocalControl {
    input: Mutex<Option<ElevatedInput>>,
    cancelled: AtomicBool,
    waiting: AtomicBool,
}

impl LocalControl {
    /// Hands over the started command's input; a cancel that came first
    /// reaches it now.
    fn attach(&self, input: ElevatedInput) {
        if self.cancelled.load(Ordering::SeqCst) {
            input.cancel();
        }
        if let Ok(mut slot) = self.input.lock() {
            *slot = Some(input);
        }
        self.waiting.store(false, Ordering::SeqCst);
    }

    fn with_input<T>(
        &self,
        work: impl FnOnce(&ElevatedInput) -> Result<T, String>,
    ) -> Result<T, String> {
        let slot = self
            .input
            .lock()
            .map_err(|_| "the command's input is poisoned".to_string())?;
        match slot.as_ref() {
            Some(input) => work(input),
            None => Err("the command has not started yet".into()),
        }
    }
}

impl JobControl for LocalControl {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.with_input(|input| {
            input.cancel();
            Ok(())
        });
    }

    fn stdin(&self, data: &[u8]) -> Result<(), String> {
        self.with_input(|input| input.stdin(data))
    }

    fn eof(&self) -> Result<(), String> {
        self.with_input(ElevatedInput::eof)
    }

    fn progress(&self) -> Value {
        if self.waiting.load(Ordering::SeqCst) {
            json!({
                "state": "waiting_for_elevation",
                "message": "the system asks the user to allow administrator or root rights (UAC, polkit)",
            })
        } else {
            Value::Null
        }
    }
}

pub(super) fn start(
    session: &Session,
    run: LocalRun,
    guard: Option<IntegrityGuard>,
) -> Result<Arc<Job>, String> {
    let job = session
        .jobs
        .create(JobKind::Shell, run.summary.clone(), &["stdout", "stderr"])?;
    let control = Arc::new(LocalControl {
        waiting: AtomicBool::new(run.elevated),
        ..LocalControl::default()
    });
    job.set_control(control.clone());
    spawn_worker(job.clone(), move |job| {
        let mut guard = guard;
        let mut end = execute(job, &control, run);
        if let Some(reason) = super::shell_tool_guard::check_after(guard.as_mut()) {
            end.status = JobStatus::Failed;
            end.error = Some(reason);
            if let Some(result) = end.result.as_object_mut() {
                result.insert(
                    "mask_store".into(),
                    json!("restored after a change from outside"),
                );
            }
        }
        end
    })?;
    Ok(job)
}

fn execute(job: &Arc<Job>, control: &Arc<LocalControl>, run: LocalRun) -> JobEnd {
    let started = if run.elevated {
        crate::elevate::spawn_elevated(run.spec)
    } else {
        crate::elevate::spawn_direct(run.spec)
    };
    let mut child = match started {
        Ok(child) => child,
        Err(error) => return refused(&error, &run.shell),
    };
    let via = child.via();
    control.attach(child.input());
    let input = child.input();
    if let Some(text) = run.stdin.as_deref().filter(|text| !text.is_empty()) {
        if let Err(error) = input.stdin(text.as_bytes()) {
            let _ = job.append(
                "stderr",
                format!("se: input not delivered: {error}\n").as_bytes(),
            );
        }
    }
    if !run.keep_stdin_open {
        let _ = input.eof();
    }
    let timed_out = Arc::new(AtomicBool::new(false));
    if let Some(seconds) = run.timeout_seconds {
        let (flag, timer, watched) = (timed_out.clone(), input.clone(), job.clone());
        let _ = std::thread::Builder::new()
            .name(format!("se-mcp-{}-timeout", job.id))
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(seconds));
                if watched.end().is_none() {
                    flag.store(true, Ordering::SeqCst);
                    timer.cancel();
                }
            });
    }
    while let Some(event) = child.next_event() {
        let appended = match event {
            ElevatedEvent::Stdout(bytes) => job.append("stdout", &bytes),
            ElevatedEvent::Stderr(bytes) => job.append("stderr", &bytes),
            ElevatedEvent::Exited(exit) => {
                return finished(
                    &exit,
                    timed_out.load(Ordering::SeqCst),
                    &run.shell,
                    via.label(),
                )
            }
        };
        if let Err(error) = appended {
            input.cancel();
            return JobEnd {
                status: JobStatus::Failed,
                result: json!({ "cli_exit_code": 125 }),
                error: Some(error),
            };
        }
    }
    JobEnd {
        status: JobStatus::Failed,
        result: json!({ "cli_exit_code": 125 }),
        error: Some("the command ended without a final status".into()),
    }
}

/// The end of a command that started; the conventions of `exec`: the exit
/// status, `128 + n` after a signal, 124 timeout, 130 cancelled, 125 when it
/// could not run.
pub(super) fn finished(exit: &ElevatedExit, timed_out: bool, shell: &str, via: &str) -> JobEnd {
    let (status, cli_exit_code) = if timed_out {
        (JobStatus::TimedOut, 124)
    } else if exit.cancelled {
        (JobStatus::Cancelled, 130)
    } else if let Some(code) = exit.status() {
        (JobStatus::Completed, code)
    } else {
        (JobStatus::Failed, 125)
    };
    JobEnd {
        status,
        result: json!({
            "exit_code": exit.code,
            "signal": exit.signal,
            "cli_exit_code": cli_exit_code,
            "shell": shell,
            "elevation": via,
        }),
        error: exit.error.clone(),
    }
}

/// A command that did not start: declined (126, as a shell reports a
/// command it may not run), no way to ask (125) or another failure (125).
pub(super) fn refused(error: &ElevateError, shell: &str) -> JobEnd {
    let (code, reason) = match error {
        ElevateError::Declined => (126, "declined"),
        ElevateError::NoAgent(_) => (125, "no_dialog"),
        ElevateError::NotAllowed(_) => (125, "not_allowed"),
        ElevateError::Failed(_) => (125, "failed"),
    };
    JobEnd {
        status: JobStatus::Failed,
        result: json!({
            "cli_exit_code": code,
            "elevation_refused": reason,
            "shell": shell,
        }),
        error: Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{finished, refused};
    use crate::cli::mcp::jobs::JobStatus;
    use crate::elevate::{ElevateError, ElevatedExit};

    #[test]
    fn shell_elevate_task_local_ends_follow_the_exec_conventions() {
        let exit = ElevatedExit {
            code: Some(3),
            ..ElevatedExit::default()
        };
        let end = finished(&exit, false, "bash", "none");
        assert_eq!(end.status, JobStatus::Completed);
        assert_eq!(end.result["cli_exit_code"], 3);
        assert_eq!(end.result["shell"], "bash");
        let killed = ElevatedExit {
            signal: Some(15),
            cancelled: true,
            ..ElevatedExit::default()
        };
        assert_eq!(
            finished(&killed, false, "bash", "none").result["cli_exit_code"],
            130
        );
        assert_eq!(
            finished(&killed, true, "bash", "none").status,
            JobStatus::TimedOut
        );
        let signalled = ElevatedExit {
            signal: Some(9),
            ..ElevatedExit::default()
        };
        assert_eq!(
            finished(&signalled, false, "sh", "polkit").result["cli_exit_code"],
            137
        );
        let declined = refused(&ElevateError::Declined, "pwsh 7");
        assert_eq!(declined.status, JobStatus::Failed);
        assert_eq!(declined.result["cli_exit_code"], 126);
        assert_eq!(declined.result["elevation_refused"], "declined");
        assert!(declined.error.unwrap().contains("declined"));
    }
}
