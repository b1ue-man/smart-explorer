//! `exec` with `session`: the command runs in a persistent shell, opened by
//! this server on first use or attached from another process, so cwd,
//! environment and shell state persist between calls. Opening runs on the job
//! thread (a Share peer may need up to a minute). Cancel and timeout end the
//! whole session: the exec stream has no signal path to a single command.
use super::exec_tool::ExecRequest;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use super::shell_registry::Port;
use crate::cli::shell_session::{
    argv_command, CommandOutcome, CommandRequest, CommandSink, OutcomeStatus, StdinMode, Stream,
};
use crate::share::ExecCommand;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(super) fn start(
    session: &Session,
    request: ExecRequest,
    name: String,
    attachable: bool,
) -> Result<Arc<Job>, String> {
    if request.keep_stdin_open {
        return Err(
            "`keep_stdin_open` is not available with `session`; give the input as `stdin`".into(),
        );
    }
    crate::cli::shell_session::validate_name(&name)?;
    let summary = shorten(format!(
        "exec in session {name}: {}",
        request.command_text()
    ));
    let job = session
        .jobs
        .create(JobKind::Exec, summary, &["stdout", "stderr"])?;
    let shells = session.shells.clone();
    let targets = session.targets.clone();
    spawn_worker(job.clone(), move |job| {
        let selector = Some(request.selector.as_str()).filter(|value| !value.trim().is_empty());
        let port = match shells.get_or_open(&name, selector, attachable, &targets) {
            Ok(port) => port,
            Err(error) => return failed(error),
        };
        run(job, &port, request, &name)
    })?;
    Ok(job)
}

fn run(job: &Arc<Job>, port: &Arc<Port>, request: ExecRequest, name: &str) -> JobEnd {
    let dialect = port.client.info().dialect;
    let command = match &request.command {
        ExecCommand::Shell { command } => command.clone(),
        ExecCommand::Argv { program, args } => argv_command(dialect, program, args),
        // MCP arguments never produce a login shell: it needs a terminal.
        ExecCommand::Login => {
            return failed("an interactive login shell needs a terminal".into());
        }
    };
    let stdin = match request.stdin.filter(|text| !text.is_empty()) {
        Some(text) => StdinMode::Text(text),
        None => StdinMode::Null,
    };
    let sink = Arc::new(JobSink {
        job: job.clone(),
        state: Mutex::new(SinkState::default()),
        done: Condvar::new(),
    });
    job.set_control(Arc::new(SessionControl(port.clone())));
    if job.cancel_requested() {
        // The cancel reached the session through `set_control` and ended it.
        let cancelled = CommandOutcome {
            status: OutcomeStatus::Cancelled,
            exit_code: None,
            cwd: None,
            session_ended: true,
            message: Some("cancelled before the command started".into()),
        };
        return job_end(&cancelled, name, false, None);
    }
    let submitted = port.client.submit(
        CommandRequest {
            command,
            cwd: request.cwd,
            env: request.env,
            stdin,
        },
        sink.clone(),
    );
    if let Err(error) = submitted {
        return failed(error);
    }
    let timed_out = Arc::new(AtomicBool::new(false));
    if let Some(seconds) = request.timeout_seconds {
        let timer_sink = sink.clone();
        let timer_flag = timed_out.clone();
        let timer_port = port.clone();
        let _ = std::thread::Builder::new()
            .name(format!("se-mcp-{}-timeout", job.id))
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(seconds));
                if !timer_sink.finished_yet() {
                    timer_flag.store(true, Ordering::SeqCst);
                    timer_port.client.cancel();
                }
            });
    }
    let (outcome, spool_error) = sink.wait();
    job_end(
        &outcome,
        name,
        timed_out.load(Ordering::SeqCst),
        spool_error,
    )
}

/// Cancel ends the session: there is no signal path to one command.
struct SessionControl(Arc<Port>);

impl JobControl for SessionControl {
    fn cancel(&self) {
        self.0.client.cancel();
    }
}

#[derive(Default)]
struct SinkState {
    outcome: Option<CommandOutcome>,
    spool_error: Option<String>,
}

struct JobSink {
    job: Arc<Job>,
    state: Mutex<SinkState>,
    done: Condvar,
}

impl CommandSink for JobSink {
    fn output(&self, stream: Stream, bytes: &[u8]) {
        let name = match stream {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        };
        if let Err(error) = self.job.append(name, bytes) {
            if let Ok(mut state) = self.state.lock() {
                state.spool_error.get_or_insert(error);
            }
        }
    }

    fn finished(&self, outcome: CommandOutcome) {
        if let Ok(mut state) = self.state.lock() {
            state.outcome = Some(outcome);
        }
        self.done.notify_all();
    }
}

impl JobSink {
    fn finished_yet(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.outcome.is_some())
            .unwrap_or(true)
    }

    /// The client finishes every sink, at the latest when the session ends
    /// or its connection is lost.
    fn wait(&self) -> (CommandOutcome, Option<String>) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        loop {
            if let Some(outcome) = state.outcome.take() {
                return (outcome, state.spool_error.take());
            }
            state = match self.done.wait(state) {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }
}

fn job_end(
    outcome: &CommandOutcome,
    name: &str,
    timed_out: bool,
    spool_error: Option<String>,
) -> JobEnd {
    let status = match outcome.status {
        _ if spool_error.is_some() => JobStatus::Failed,
        OutcomeStatus::Completed => JobStatus::Completed,
        OutcomeStatus::Cancelled if timed_out => JobStatus::TimedOut,
        OutcomeStatus::Cancelled => JobStatus::Cancelled,
        OutcomeStatus::Failed => JobStatus::Failed,
    };
    let cli_exit_code = match status {
        JobStatus::Completed => outcome.exit_code.unwrap_or(125),
        JobStatus::TimedOut => 124,
        JobStatus::Cancelled => 130,
        JobStatus::Failed => 125,
    };
    JobEnd {
        status,
        result: json!({
            "session": name,
            "exit_code": outcome.exit_code,
            "cwd": outcome.cwd,
            "session_ended": outcome.session_ended,
            "message": outcome.message,
            "cli_exit_code": cli_exit_code,
        }),
        error: match spool_error {
            Some(error) => Some(error),
            None if status == JobStatus::Failed => outcome.message.clone(),
            None => None,
        },
    }
}

fn failed(error: String) -> JobEnd {
    JobEnd {
        status: JobStatus::Failed,
        result: json!({ "cli_exit_code": 125 }),
        error: Some(error),
    }
}

fn shorten(summary: String) -> String {
    if summary.chars().count() <= 160 {
        return summary;
    }
    summary
        .chars()
        .take(159)
        .chain(std::iter::once('…'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::job_end;
    use crate::cli::mcp::jobs::JobStatus;
    use crate::cli::shell_session::{CommandOutcome, OutcomeStatus};

    #[test]
    fn mcp_task_session_outcomes_map_to_job_states() {
        let done = CommandOutcome {
            status: OutcomeStatus::Completed,
            exit_code: Some(3),
            cwd: Some("/srv".into()),
            session_ended: false,
            message: None,
        };
        let end = job_end(&done, "build", false, None);
        assert_eq!(end.status, JobStatus::Completed);
        assert_eq!(end.result["cli_exit_code"], 3);
        assert_eq!(end.result["cwd"], "/srv");
        assert_eq!(end.result["session"], "build");
        let cancelled = CommandOutcome {
            status: OutcomeStatus::Cancelled,
            exit_code: None,
            cwd: None,
            session_ended: true,
            message: Some("the session was cancelled by se shell (pid 1)".into()),
        };
        assert_eq!(
            job_end(&cancelled, "b", true, None).status,
            JobStatus::TimedOut
        );
        assert_eq!(
            job_end(&cancelled, "b", true, None).result["cli_exit_code"],
            124
        );
        assert_eq!(
            job_end(&cancelled, "b", false, None).status,
            JobStatus::Cancelled
        );
        let lost = CommandOutcome {
            status: OutcomeStatus::Failed,
            exit_code: None,
            cwd: None,
            session_ended: true,
            message: Some("lost".into()),
        };
        let end = job_end(&lost, "b", false, None);
        assert_eq!(end.status, JobStatus::Failed);
        assert_eq!(end.error.as_deref(), Some("lost"));
        let spool = job_end(&done, "b", false, Some("disk full".into()));
        assert_eq!(spool.status, JobStatus::Failed);
        assert_eq!(spool.error.as_deref(), Some("disk full"));
    }
}
