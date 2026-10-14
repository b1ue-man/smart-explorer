//! `exec`: a command on a saved SFTP remote (SSH login) or on a Share peer
//! that granted this device Exec, run as a job with complete stdout/stderr;
//! with `session` in a persistent shell (`exec_session_job.rs`).
use super::args::Args;
use super::jobs::Job;
use super::session::Session;
use crate::share::ExecCommand;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) struct ExecRequest {
    pub(super) selector: String,
    pub(super) command: ExecCommand,
    pub(super) cwd: Option<String>,
    pub(super) env: BTreeMap<String, String>,
    pub(super) stdin: Option<String>,
    pub(super) keep_stdin_open: bool,
    pub(super) timeout_seconds: Option<u64>,
}

impl ExecRequest {
    /// The command as one line, for summaries.
    pub(super) fn command_text(&self) -> String {
        match &self.command {
            ExecCommand::Shell { command } => command.clone(),
            ExecCommand::Argv { program, args } => std::iter::once(program.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    pub(super) fn summary(&self) -> String {
        let mut summary = format!("exec on {}: {}", self.selector, self.command_text());
        if summary.chars().count() > 160 {
            summary = summary
                .chars()
                .take(159)
                .chain(std::iter::once('…'))
                .collect();
        }
        summary
    }
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let command =
        match (args.opt_string("command")?, args.opt_strings("argv")?) {
            (Some(_), Some(_)) => return Err("give either `command` or `argv`, not both".into()),
            (Some(command), None) if !command.trim().is_empty() => ExecCommand::Shell { command },
            (None, Some(argv)) if !argv.is_empty() => ExecCommand::Argv {
                program: argv[0].clone(),
                args: argv[1..].to_vec(),
            },
            _ => return Err(
                "missing `command` (one shell string) or `argv` (program and literal arguments)"
                    .into(),
            ),
        };
    let session_name = args
        .opt_string("session")?
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    // An open session needs no target; opening one does.
    let selector = match &session_name {
        Some(_) => args.opt_string("target")?.unwrap_or_default(),
        None => args.string("target")?,
    };
    let request = ExecRequest {
        selector,
        command,
        cwd: args.opt_string("cwd")?.filter(|cwd| !cwd.is_empty()),
        env: args.string_map("env")?,
        stdin: args.opt_string("stdin")?,
        keep_stdin_open: args.flag("keep_stdin_open")?,
        timeout_seconds: args
            .opt_u64("timeout_seconds")?
            .filter(|seconds| *seconds > 0),
    };
    if let Some(name) = session_name {
        let attachable = match args.value("attachable") {
            None => true,
            Some(_) => args.flag("attachable")?,
        };
        return super::exec_session_job::start(session, request, name, attachable);
    }
    match crate::cli::exec_ssh::ssh_target(&request.selector)? {
        Some(target) => super::exec_ssh_job::start(session, target, request),
        None => super::exec_share_job::start(session, request),
    }
}
