//! `exec` on a saved SFTP remote over its SSH login (`sftp/core/command.rs`):
//! a timeout or job_cancel sends TERM, then KILL after 10 s, to the command's
//! process group. `elevated` runs it through `sudo` (`exec_ssh_sudo.rs`):
//! that start happens in the job's worker, because a password may be asked
//! from the user first (`{"state":"waiting_for_sudo"}` meanwhile).
use super::exec_tool::ExecRequest;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use crate::cli::exec_peer::NO_SHELL;
use crate::cli::exec_ssh::sudo::Sudo;
use crate::cli::exec_ssh::{exit_code, remote_command_line, SshExecTarget, SSH_LOGIN_SHELL};
use crate::sftp::{SftpBackend, SshCommand, SshCommandControl, SshCommandEnd, SshCommandEvent};
use crate::share::ExecCommand;
use serde_json::{json, Value};
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

/// The control of an elevated start before its command runs.
struct WaitingForSudo;

impl JobControl for WaitingForSudo {
    fn cancel(&self) {}

    fn progress(&self) -> Value {
        json!({ "state": "waiting_for_sudo" })
    }
}

pub(super) fn start(
    session: &Session,
    target: SshExecTarget,
    request: ExecRequest,
) -> Result<Arc<Job>, String> {
    if request.elevated {
        return start_elevated(session, target, request);
    }
    let summary = request.summary();
    let shell = result_shell(&request.command);
    let cwd = request.cwd.clone().or(target.cwd.clone());
    let line = remote_command_line(&request.command, cwd.as_deref(), &request.env)?;
    let backend = session.targets.ssh(&target.connection)?;
    let command = backend.start_command(&line).map_err(|error| {
        // The next command logs in again instead of reusing a broken login.
        session.targets.ssh_failed(&target.connection);
        format!("remote command start: {error}")
    })?;
    let control = command.control();
    let job = session
        .jobs
        .create(JobKind::Exec, summary, &["stdout", "stderr"])?;
    job.set_control(Arc::new(SshControl(control.clone())));
    feed(&control, None, &request)?;
    let timed_out = arm_timeout(&job, &control, request.timeout_seconds)?;
    spawn_worker(job.clone(), move |job| {
        drive(command, &control, job, &timed_out, shell, None)
    })?;
    Ok(job)
}

/// `exec` with `elevated` on an SSH remote: sudo is prepared (and its
/// password asked) inside the job.
fn start_elevated(
    session: &Session,
    target: SshExecTarget,
    request: ExecRequest,
) -> Result<Arc<Job>, String> {
    let job = session
        .jobs
        .create(JobKind::Exec, request.summary(), &["stdout", "stderr"])?;
    job.set_control(Arc::new(WaitingForSudo));
    let targets = session.targets.clone();
    spawn_worker(job.clone(), move |job| {
        match run_elevated(&targets, &target, &request, job) {
            Ok(end) => end,
            Err(error) => JobEnd {
                status: if job.cancel_requested() {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                },
                result: json!({ "cli_exit_code": 125, "elevation": "sudo" }),
                error: Some(error),
            },
        }
    })?;
    Ok(job)
}

fn run_elevated(
    targets: &super::targets::Targets,
    target: &SshExecTarget,
    request: &ExecRequest,
    job: &Arc<Job>,
) -> Result<JobEnd, String> {
    let cwd = request.cwd.clone().or(target.cwd.clone());
    let line = remote_command_line(&request.command, cwd.as_deref(), &request.env)?;
    let backend: Arc<SftpBackend> = targets.ssh(&target.connection)?;
    let sudo = crate::cli::exec_ssh::sudo::prepare(
        &backend,
        &target.connection.display(),
        &crate::cli::elevated::password::ask,
    )?;
    if job.cancel_requested() {
        return Err("cancelled before the elevated command started".into());
    }
    let command = backend.start_command(&sudo.wrap(&line)).map_err(|error| {
        targets.ssh_failed(&target.connection);
        format!("remote command start: {error}")
    })?;
    let control = command.control();
    job.set_control(Arc::new(SshControl(control.clone())));
    feed(&control, Some(&sudo), request)?;
    let timed_out = arm_timeout(job, &control, request.timeout_seconds)?;
    Ok(drive(
        command,
        &control,
        job,
        &timed_out,
        result_shell(&request.command),
        Some(sudo.label()),
    ))
}

fn result_shell(command: &ExecCommand) -> &'static str {
    match command {
        ExecCommand::Shell { .. } => SSH_LOGIN_SHELL,
        _ => NO_SHELL,
    }
}

/// The sudo password line, `stdin` and its end unless `keep_stdin_open`.
fn feed(
    control: &SshCommandControl,
    sudo: Option<&Sudo>,
    request: &ExecRequest,
) -> Result<(), String> {
    if let Some(prefix) = sudo.and_then(Sudo::input_prefix) {
        control.stdin(&prefix).map_err(|error| error.to_string())?;
    }
    if let Some(text) = request.stdin.as_deref().filter(|text| !text.is_empty()) {
        control
            .stdin(text.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    if !request.keep_stdin_open {
        control.eof().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn arm_timeout(
    job: &Arc<Job>,
    control: &SshCommandControl,
    seconds: Option<u64>,
) -> Result<Arc<AtomicBool>, String> {
    let timed_out = Arc::new(AtomicBool::new(false));
    if let Some(seconds) = seconds {
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
    Ok(timed_out)
}

fn drive(
    mut command: SshCommand,
    control: &SshCommandControl,
    job: &Arc<Job>,
    timed_out: &AtomicBool,
    shell: &str,
    elevation: Option<&str>,
) -> JobEnd {
    while let Some(event) = command.next_event() {
        let appended = match event {
            SshCommandEvent::Started => Ok(()),
            SshCommandEvent::Stdout(bytes) => job.append("stdout", &bytes),
            SshCommandEvent::Stderr(bytes) => job.append("stderr", &bytes),
            SshCommandEvent::Finished(end) => {
                let mut end = command_end(&end, timed_out.load(Ordering::SeqCst), shell);
                if let (Some(elevation), Some(result)) = (elevation, end.result.as_object_mut()) {
                    result.insert("elevation".into(), json!(elevation));
                }
                return end;
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
}

/// `shell`: the result name of the shell (`ssh login shell`, `none` for argv).
fn command_end(end: &SshCommandEnd, timed_out: bool, shell: &str) -> JobEnd {
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
            "shell": shell,
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
        assert_eq!(
            command_end(&done, false, "ssh login shell").status,
            JobStatus::Completed
        );
        assert_eq!(
            command_end(&done, false, "ssh login shell").result["shell"],
            "ssh login shell"
        );
        let killed = SshCommandEnd {
            signal: Some("TERM".into()),
            cancelled: true,
            ..Default::default()
        };
        assert_eq!(
            command_end(&killed, false, "none").status,
            JobStatus::Cancelled
        );
        assert_eq!(
            command_end(&killed, true, "none").status,
            JobStatus::TimedOut
        );
        assert_eq!(
            command_end(&killed, true, "none").result["cli_exit_code"],
            124
        );
        let refused = SshCommandEnd {
            error: Some("rejected".into()),
            ..Default::default()
        };
        assert_eq!(
            command_end(&refused, false, "none").status,
            JobStatus::Failed
        );
    }
}
