//! `exec`: a command on a saved SFTP remote (SSH login) or on a Share peer
//! that granted this device Exec, run as a job with complete stdout/stderr.
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
    pub(super) fn summary(&self) -> String {
        let command = match &self.command {
            ExecCommand::Shell { command } => command.clone(),
            ExecCommand::Argv { program, args } => std::iter::once(program.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
        };
        let mut summary = format!("exec on {}: {command}", self.selector);
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
    let request = ExecRequest {
        selector: args.string("target")?,
        command,
        cwd: args.opt_string("cwd")?.filter(|cwd| !cwd.is_empty()),
        env: args.string_map("env")?,
        stdin: args.opt_string("stdin")?,
        keep_stdin_open: args.flag("keep_stdin_open")?,
        timeout_seconds: args
            .opt_u64("timeout_seconds")?
            .filter(|seconds| *seconds > 0),
    };
    match crate::cli::exec_ssh::ssh_target(&request.selector)? {
        Some(target) => super::exec_ssh_job::start(session, target, request),
        None => super::exec_share_job::start(session, request),
    }
}
