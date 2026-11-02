//! `exec` on a Share peer: the background worker's Exec stream (the same path
//! as `se exec PEER`), with the contained process tree and its lifecycle on the
//! peer (docs/REMOTE_EXECUTION_PLAN.md). While the peer asks its user to
//! confirm the start (PRM-M3), the running job's progress is
//! `{"state":"waiting_for_approval",…}`; once authorized it names the account
//! the command runs as.
use super::exec_tool::ExecRequest;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use crate::daemon::{ExecIpcEvent, ExecIpcInput, ExecIpcSession, ExecNoticeWatch};
use crate::share::exec_client_messages::{account_line, approval_status_line};
use crate::share::{ExecId, ExecTerminal, ExecTerminalKind};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What the job status tells beside the output while the start runs.
struct ShareProgress {
    /// The peer as the request selected it.
    label: String,
    /// Reads the start's notice until it begins.
    watch: Mutex<Option<ExecNoticeWatch>>,
    /// `{device}: as {account}` once the peer authorized the start.
    account: Mutex<Option<String>>,
}

impl ShareProgress {
    fn device(&self, named: &str) -> String {
        if named.trim().is_empty() {
            self.label.clone()
        } else {
            named.to_string()
        }
    }

    fn authorized(&self, exec_id: &ExecId, user_label: &str) {
        let notice = crate::daemon::exec_notice(exec_id).ok().flatten();
        let device = self.device(notice.as_ref().map_or("", |notice| notice.device.as_str()));
        let access = notice.as_ref().and_then(|notice| notice.access.as_ref());
        *lock(&self.account) = Some(account_line(&device, access, user_label));
    }
}

fn lock<T>(slot: &Mutex<T>) -> MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

struct ShareControl {
    input: ExecIpcInput,
    progress: Arc<ShareProgress>,
}

impl JobControl for ShareControl {
    fn cancel(&self) {
        let _ = self.input.cancel();
    }

    fn stdin(&self, data: &[u8]) -> Result<(), String> {
        self.input.stdin(data).map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        self.input.eof().map_err(|error| error.to_string())
    }

    fn progress(&self) -> Value {
        let notice = lock(&self.progress.watch)
            .as_ref()
            .and_then(ExecNoticeWatch::latest);
        if let Some(notice) = notice {
            if let Some(approval) = &notice.approval {
                let device = self.progress.device(&notice.device);
                return json!({
                    "state": "waiting_for_approval",
                    "device": device,
                    "surface": approval.surface,
                    "message": approval_status_line(&device, approval.surface()),
                });
            }
        }
        match lock(&self.progress.account).clone() {
            Some(line) => json!({ "account": line }),
            None => Value::Null,
        }
    }
}

pub(super) fn start(session: &Session, request: ExecRequest) -> Result<Arc<Job>, String> {
    let summary = request.summary();
    let label = request.selector.clone();
    let peer = crate::cli::exec::select_target(Some(request.selector.as_str()))?;
    let start = crate::share::ExecStart {
        exec_id: crate::share::ExecId::generate().map_err(|error| error.to_string())?,
        command: request.command,
        cwd: request.cwd,
        env: request.env,
        timeout_ms: request
            .timeout_seconds
            .map(|seconds| seconds.saturating_mul(1_000)),
        max_output_bytes: None,
        terminal: None,
    };
    start.validate().map_err(|error| error.to_string())?;
    let remote = crate::daemon::connect_exec(peer, start)?;
    let input = remote.take_input().map_err(|error| error.to_string())?;
    let job = session
        .jobs
        .create(JobKind::Exec, summary, &["stdout", "stderr"])?;
    // Every new notice wakes the waiters of `job_status`.
    let watched = job.clone();
    let watch = ExecNoticeWatch::start(remote.exec_id.clone(), move |_| watched.changed());
    let progress = Arc::new(ShareProgress {
        label,
        watch: Mutex::new(Some(watch)),
        account: Mutex::new(None),
    });
    job.set_control(Arc::new(ShareControl {
        input: input.clone(),
        progress: progress.clone(),
    }));
    if let Some(text) = request.stdin.as_deref().filter(|text| !text.is_empty()) {
        input
            .stdin(text.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    if !request.keep_stdin_open {
        input.eof().map_err(|error| error.to_string())?;
    }
    spawn_worker(job.clone(), move |job| {
        drive(remote, &input, job, &progress)
    })?;
    Ok(job)
}

fn drive(
    mut remote: ExecIpcSession,
    input: &ExecIpcInput,
    job: &Arc<Job>,
    progress: &ShareProgress,
) -> JobEnd {
    loop {
        let event = match remote.next_event() {
            Ok(event) => event,
            Err(error) => return failed(format!("remote execution transport: {error}")),
        };
        // Started, output or the end: nothing waits for the peer's user.
        if !matches!(event, ExecIpcEvent::Authorized(_)) && lock(&progress.watch).take().is_some() {
            job.changed();
        }
        let appended = match event {
            ExecIpcEvent::Authorized(provider) => {
                progress.authorized(&remote.exec_id, &provider.user_label);
                job.changed();
                Ok(())
            }
            ExecIpcEvent::Started => Ok(()),
            ExecIpcEvent::Stdout(bytes) => job.append("stdout", &bytes),
            ExecIpcEvent::Stderr(bytes) => job.append("stderr", &bytes),
            ExecIpcEvent::Failed(failure) => {
                return JobEnd {
                    status: JobStatus::Failed,
                    result: json!({ "code": failure.code, "kind": failure.kind }),
                    error: Some(failure.message),
                }
            }
            ExecIpcEvent::Terminal(terminal) => return terminal_end(&terminal),
        };
        if let Err(error) = appended {
            let _ = input.cancel();
            return failed(error);
        }
    }
}

fn failed(error: String) -> JobEnd {
    JobEnd {
        status: JobStatus::Failed,
        result: json!({ "cli_exit_code": 125 }),
        error: Some(error),
    }
}

pub(super) fn terminal_end(terminal: &ExecTerminal) -> JobEnd {
    let status = match terminal.kind {
        ExecTerminalKind::Exited => JobStatus::Completed,
        ExecTerminalKind::TimedOut => JobStatus::TimedOut,
        ExecTerminalKind::Cancelled => JobStatus::Cancelled,
        ExecTerminalKind::Failed | ExecTerminalKind::Revoked | ExecTerminalKind::Disconnected => {
            JobStatus::Failed
        }
    };
    let cli_exit_code = match terminal.kind {
        ExecTerminalKind::Exited => terminal
            .exit_code
            .or_else(|| terminal.signal.map(|signal| 128 + signal))
            .unwrap_or(1),
        ExecTerminalKind::TimedOut => 124,
        ExecTerminalKind::Cancelled => 130,
        _ => 125,
    };
    JobEnd {
        status,
        result: json!({
            "outcome": serde_json::to_value(&terminal.kind).unwrap_or_default(),
            "exit_code": terminal.exit_code,
            "signal": terminal.signal,
            "cli_exit_code": cli_exit_code,
            "output_truncated": terminal.output_truncated,
        }),
        error: (status == JobStatus::Failed)
            .then(|| terminal.message.clone())
            .flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::terminal_end;
    use crate::cli::mcp::jobs::JobStatus;
    use crate::share::{ExecId, ExecTerminal, ExecTerminalKind};

    #[test]
    fn mcp_task_share_exec_terminals_map_to_job_states() {
        let mut terminal = ExecTerminal {
            exec_id: ExecId::parse("00".repeat(16)).unwrap(),
            kind: ExecTerminalKind::Exited,
            exit_code: Some(3),
            signal: None,
            message: None,
            stdout_bytes: 1,
            stderr_bytes: 0,
            output_truncated: false,
        };
        let end = terminal_end(&terminal);
        assert_eq!(end.status, JobStatus::Completed);
        assert_eq!(end.result["exit_code"], 3);
        assert_eq!(end.result["cli_exit_code"], 3);
        terminal.kind = ExecTerminalKind::Revoked;
        terminal.message = Some("grant removed".into());
        let end = terminal_end(&terminal);
        assert_eq!(end.status, JobStatus::Failed);
        assert_eq!(end.error.as_deref(), Some("grant removed"));
        assert_eq!(end.result["cli_exit_code"], 125);
    }
}
