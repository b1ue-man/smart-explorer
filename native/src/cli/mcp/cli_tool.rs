//! The `se` tool: any other terminal command (`se <args>`) as a child process
//! of the installed `se`, so every present and future CLI function is
//! reachable with exactly its terminal behavior. Output is captured
//! completely; cancellation asks the child to stop like Ctrl+C (Unix SIGINT),
//! then terminates it after 10 seconds. Windows children run without a
//! console window and are terminated directly.
use super::args::Args;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use serde_json::json;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Time a child gets after the stop request before it is terminated.
const STOP_GRACE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

/// The worker polls the job's cancel flag and stops the child itself.
struct CliControl;

impl JobControl for CliControl {
    fn cancel(&self) {}
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let arguments = args
        .opt_strings("args")?
        .filter(|arguments| !arguments.is_empty())
        .ok_or_else(|| "missing `args`, the arguments after `se`".to_string())?;
    if arguments
        .first()
        .is_some_and(|first| first.eq_ignore_ascii_case("mcp"))
    {
        return Err("`se mcp` is this server; it cannot run inside itself".into());
    }
    let stdin = args.opt_string("stdin")?;
    let mut command = Command::new(&session.executable);
    command
        .args(&arguments)
        .env_remove("COMPLETE")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::os::configure_child(&mut command);
    let job = session.jobs.create(
        JobKind::Command,
        format!("se {}", arguments.join(" ")),
        &["stdout", "stderr"],
    )?;
    job.set_control(Arc::new(CliControl));
    let not_started = |error: String| {
        job.finish(JobEnd {
            status: JobStatus::Failed,
            result: json!({}),
            error: Some(error.clone()),
        });
        error
    };
    let mut child = command.spawn().map_err(|error| {
        not_started(format!(
            "cannot start {}: {error}",
            session.executable.display()
        ))
    })?;
    let readers = match capture(&job, &mut child, stdin) {
        Ok(readers) => readers,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(not_started(error));
        }
    };
    spawn_worker(job.clone(), move |job| {
        let mut child = child;
        let mut stop_deadline = None;
        let status = loop {
            if job.cancel_requested() && stop_deadline.is_none() {
                let graceful = super::os::request_child_stop(&child);
                stop_deadline =
                    Some(Instant::now() + if graceful { STOP_GRACE } else { Duration::ZERO });
            }
            if stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(POLL),
                Err(error) => break Err(error),
            }
        };
        for reader in readers {
            let _ = reader.join();
        }
        match status {
            Ok(status) => JobEnd {
                status: if job.cancel_requested() {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Completed
                },
                result: json!({ "exit_code": status.code() }),
                error: None,
            },
            Err(error) => JobEnd {
                status: JobStatus::Failed,
                result: json!({}),
                error: Some(format!("cannot wait for the se child: {error}")),
            },
        }
    })?;
    Ok(job)
}

/// Feeds the optional standard input and starts the output readers.
fn capture(
    job: &Arc<Job>,
    child: &mut std::process::Child,
    stdin: Option<String>,
) -> Result<Vec<std::thread::JoinHandle<()>>, String> {
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        std::thread::Builder::new()
            .name(format!("se-mcp-{}-stdin", job.id))
            .spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
            })
            .map_err(|error| format!("cannot feed standard input: {error}"))?;
    }
    let mut readers = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        readers.push(copy_stream(job, "stdout", pipe)?);
    }
    if let Some(pipe) = child.stderr.take() {
        readers.push(copy_stream(job, "stderr", pipe)?);
    }
    Ok(readers)
}

fn copy_stream(
    job: &Arc<Job>,
    name: &'static str,
    mut pipe: impl Read + Send + 'static,
) -> Result<std::thread::JoinHandle<()>, String> {
    let job = job.clone();
    std::thread::Builder::new()
        .name(format!("se-mcp-{}-{name}", job.id))
        .spawn(move || {
            let mut buffer = [0u8; 64 * 1024];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => return,
                    Ok(read) => {
                        if job.append(name, &buffer[..read]).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => return,
                }
            }
        })
        .map_err(|error| format!("cannot capture {name}: {error}"))
}
