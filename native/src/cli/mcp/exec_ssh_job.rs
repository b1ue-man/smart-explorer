//! `exec` on a saved SFTP remote over its SSH login (`sftp/core/command.rs`):
//! a timeout or job_cancel sends TERM, then KILL after 10 s, to the command's
//! process group.
use super::exec_tool::ExecRequest;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use crate::cli::exec_ssh::{exit_code, remote_command_line, SshExecTarget};
use crate::sftp::{SshCommandControl, SshCommandEnd, SshCommandEvent};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct SshControl(SshCommandControl);

impl JobControl for SshControl {
    fn cancel(&self) {
        self.0.cancel();
    }

    fn stdin(&self, data: &[u8]) -> Result<(), String> {
        self.0.stdin(data).map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        self.0.eof().map_err(|error| error.to_string())
    }
}

pub(super) fn start(
    session: &Session,
    target: SshExecTarget,
    request: ExecRequest,
) -> Result<Arc<Job>, String> {
    let summary = request.summary();
    let cwd = request.cwd.clone().or(target.cwd.clone());
    let line = remote_command_line(&request.command, cwd.as_deref(), &request.env)?;
    let backend = session.targets.ssh(&target.connection)?;
    let mut command = backend
        .start_command(&line)
        .map_err(|error| format!("remote command start: {error}"))?;
    let control = command.control();
    let job = session
        .jobs
        .create(JobKind::Exec, summary, &["stdout", "stderr"])?;
    job.set_control(Arc::new(SshControl(control.clone())));
    if let Some(text) = request.stdin.as_deref().filter(|text| !text.is_empty()) {
        control
            .stdin(text.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    if !request.keep_stdin_open {
        control.eof().map_err(|error| error.to_string())?;
    }
    let timed_out = Arc::new(AtomicBool::new(false));
    if let Some(seconds) = request.timeout_seconds {
        let timer_job = job.clone();
        let timer_flag = timed_out.clone();
        let timer_control = control.clone();
        std::thread::Builder::new()
            .name(format!("se-mcp-{}-timeout", job.id))
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(seconds));
                if timer_job.end().is_none() {
                    timer_flag.store(true, Ordering::SeqCst);
                    timer_control.cancel();
                }
            })
            .map_err(|error| format!("cannot start the timeout timer: {error}"))?;
    }
    spawn_worker(job.clone(), move |job| {
        while let Some(event) = command.next_event() {
            let appended = match event {
                SshCommandEvent::Started => Ok(()),
                SshCommandEvent::Stdout(bytes) => job.append("stdout", &bytes),
                SshCommandEvent::Stderr(bytes) => job.append("stderr", &bytes),
                SshCommandEvent::Finished(end) => {
                    return command_end(&end, timed_out.load(Ordering::SeqCst))
                }
            };
            if let Err(error) = appended {
                control.cancel();
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
            error: Some("the remote command ended without a final status".into()),
        }
    })?;
    Ok(job)
}

fn command_end(end: &SshCommandEnd, timed_out: bool) -> JobEnd {
    let timed_out = timed_out && end.exit_status.is_none();
    let status = if timed_out {
        JobStatus::TimedOut
    } else if end.cancelled {
        JobStatus::Cancelled
    } else if end.exit_status.is_none() && end.signal.is_none() {
        JobStatus::Failed
    } else {
        JobStatus::Completed
    };
    JobEnd {
        status,
        result: json!({
            "exit_code": end.exit_status,
            "signal": end.signal,
            "cli_exit_code": exit_code(end, timed_out),
        }),
        error: end.error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::command_end;
    use crate::cli::mcp::jobs::JobStatus;
    use crate::sftp::SshCommandEnd;

    #[test]
    fn mcp_task_ssh_command_ends_map_to_job_states() {
        let done = SshCommandEnd {
            exit_status: Some(0),
            ..Default::default()
        };
        assert_eq!(command_end(&done, false).status, JobStatus::Completed);
        let killed = SshCommandEnd {
            signal: Some("TERM".into()),
            cancelled: true,
            ..Default::default()
        };
        assert_eq!(command_end(&killed, false).status, JobStatus::Cancelled);
        assert_eq!(command_end(&killed, true).status, JobStatus::TimedOut);
        assert_eq!(command_end(&killed, true).result["cli_exit_code"], 124);
        let refused = SshCommandEnd {
            error: Some("rejected".into()),
            ..Default::default()
        };
        assert_eq!(command_end(&refused, false).status, JobStatus::Failed);
    }
}
