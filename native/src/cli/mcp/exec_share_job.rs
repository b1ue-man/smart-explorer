//! `exec` on a Share peer: the background worker's Exec stream (the same path
//! as `se exec PEER`), with the contained process tree and its lifecycle on the
//! peer (docs/REMOTE_EXECUTION_PLAN.md).
use super::exec_tool::ExecRequest;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use crate::daemon::{ExecIpcEvent, ExecIpcInput, ExecIpcSession};
use crate::share::{ExecTerminal, ExecTerminalKind};
use serde_json::json;
use std::sync::Arc;

struct ShareControl(ExecIpcInput);

impl JobControl for ShareControl {
    fn cancel(&self) {
        let _ = self.0.cancel();
    }

    fn stdin(&self, data: &[u8]) -> Result<(), String> {
        self.0.stdin(data).map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        self.0.eof().map_err(|error| error.to_string())
    }
}

pub(super) fn start(session: &Session, request: ExecRequest) -> Result<Arc<Job>, String> {
    let summary = request.summary();
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
    job.set_control(Arc::new(ShareControl(input.clone())));
    if let Some(text) = request.stdin.as_deref().filter(|text| !text.is_empty()) {
        input
            .stdin(text.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    if !request.keep_stdin_open {
        input.eof().map_err(|error| error.to_string())?;
    }
    spawn_worker(job.clone(), move |job| drive(remote, &input, job))?;
    Ok(job)
}

fn drive(mut remote: ExecIpcSession, input: &ExecIpcInput, job: &Arc<Job>) -> JobEnd {
    loop {
        let event = match remote.next_event() {
            Ok(event) => event,
            Err(error) => return failed(format!("remote execution transport: {error}")),
        };
        let appended = match event {
            ExecIpcEvent::Authorized(_) | ExecIpcEvent::Started => Ok(()),
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
