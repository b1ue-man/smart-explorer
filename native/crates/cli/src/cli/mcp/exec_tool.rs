//! `exec`: a command on a saved SFTP remote (SSH login) or on a Share peer
//! that granted this device Exec, run as a job with complete stdout/stderr;
//! with `session` in a persistent shell (`exec_session_job.rs`).
use super::args::Args;
use super::jobs::Job;
use super::session::Session;
use crate::share::{ExecCommand, ExecShell};
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
    /// The shell `shell` names; `None`: the target's own (PowerShell on
    /// Windows peers, the login shell elsewhere).
    pub(super) shell: Option<ExecShell>,
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
            ExecCommand::Login => "<shell>".to_string(),
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
    let shell = args
        .opt_string("shell")?
        .map(|value| ExecShell::parse(&value))
        .transpose()?;
    if shell.is_some() && !matches!(command, ExecCommand::Shell { .. }) {
        return Err(
            "`shell` chooses the shell of `command`; `argv` starts the program without one".into(),
        );
    }
    let session_name = args
        .opt_string("session")?
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    let explicit = args
        .opt_string("target")?
        .map(|target| target.trim().to_string())
        .filter(|target| !target.is_empty());
    let cwd = args.opt_string("cwd")?.filter(|cwd| !cwd.is_empty());
    // An open session needs no target; opening one does. Without a target
    // the command runs on the working root's SSH remote or Share peer.
    let (selector, cwd) = match (explicit, &session_name) {
        (Some(target), _) => (target, cwd),
        (None, Some(name)) if session.shells.is_open(name) => (String::new(), cwd),
        (None, _) => match session.targets.root() {
            // A session keeps its own working directory between commands.
            Some(root) if session_name.is_some() => {
                (super::root_tool::exec_home(&root)?.selector, cwd)
            }
            Some(root) => root_home(&root, cwd)?,
            None if session_name.is_some() => (String::new(), cwd),
            None => {
                return Err(
                    "missing required argument `target` (or set a working root with `root`)".into(),
                )
            }
        },
    };
    let request = ExecRequest {
        selector,
        command,
        cwd,
        env: args.string_map("env")?,
        stdin: args.opt_string("stdin")?,
        keep_stdin_open: args.flag("keep_stdin_open")?,
        timeout_seconds: args
            .opt_u64("timeout_seconds")?
            .filter(|seconds| *seconds > 0),
        shell,
    };
    if let Some(name) = session_name {
        let attachable = match args.value("attachable") {
            None => true,
            Some(_) => args.flag("attachable")?,
        };
        return super::exec_session_job::start(session, request, name, attachable);
    }
    match crate::cli::exec_ssh::ssh_target(&request.selector)? {
        Some(target) => {
            crate::cli::exec_ssh::check_shell(request.shell)?;
            super::exec_ssh_job::start(session, target, request)
        }
        None => super::exec_share_job::start(session, request),
    }
}

/// The `exec` target and working directory for the working root: an SSH
/// root runs in its path (a relative `cwd` below it), a Share root on its
/// peer with the `cwd` given, since the peer does not translate a Share
/// folder into its own disk path.
fn root_home(
    root: &super::root_path::WorkRoot,
    cwd: Option<String>,
) -> Result<(String, Option<String>), String> {
    let home = super::root_tool::exec_home(root)?;
    if let Some(share_path) = home.share_path {
        return match cwd {
            Some(cwd) => Ok((home.selector, Some(cwd))),
            None => Err(format!(
                "the working root {} is the Share folder {share_path} on that device; Share does not tell this device the folder's path on the peer's disk, so `exec` cannot start there by itself: pass `cwd` with the peer's own path, or give `target`",
                root.spec
            )),
        };
    }
    let cwd = match cwd {
        // Relative to the root's folder on the SSH remote.
        Some(cwd) if remote_relative(&cwd) && !root.path.is_empty() => {
            let rel = super::root_path::normalize_relative(&cwd, &root.spec)?;
            Some(crate::cli::ops::join(&root.path, &rel))
        }
        other => other,
    };
    Ok((home.selector, cwd))
}

/// A working directory on the remote that names no root of its own: not
/// `/…`, `\\…`, `~…`, a drive (`C:…`) or a variable (`$HOME`, `%USERPROFILE%`).
fn remote_relative(cwd: &str) -> bool {
    let drive = cwd.as_bytes().get(1) == Some(&b':');
    !(drive || cwd.starts_with(['/', '\\', '~', '$', '%']))
}

#[cfg(test)]
mod tests {
    use super::{remote_relative, root_home};
    use crate::cli::mcp::root_path::{RootKind, WorkRoot};
    use crate::creds::Protocol;

    #[test]
    fn mcp_root_task_exec_without_target_runs_at_the_working_root() {
        let ssh = WorkRoot {
            spec: "@prod:/srv/app".into(),
            kind: RootKind::Login(Protocol::Sftp),
            path: "/srv/app".into(),
        };
        // The root itself is the target; its path is the working directory.
        assert_eq!(
            root_home(&ssh, None).unwrap(),
            ("@prod:/srv/app".into(), None)
        );
        assert_eq!(
            root_home(&ssh, Some("src/../bin".into()))
                .unwrap()
                .1
                .as_deref(),
            Some("/srv/app/bin")
        );
        assert_eq!(
            root_home(&ssh, Some("/tmp".into())).unwrap().1.as_deref(),
            Some("/tmp")
        );
        assert!(root_home(&ssh, Some("..".into())).is_err());
        let peer = WorkRoot {
            spec: "share://direct/c1/Docs/proj".into(),
            kind: RootKind::Peer,
            path: "/Docs/proj".into(),
        };
        let refused = root_home(&peer, None).unwrap_err();
        assert!(refused.contains("cwd"), "{refused}");
        assert_eq!(
            root_home(&peer, Some("C:\\Users\\a".into())).unwrap(),
            ("share://direct/c1".into(), Some("C:\\Users\\a".into()))
        );
        let top = WorkRoot {
            spec: "share://room/r1/d1/".into(),
            kind: RootKind::Peer,
            path: "/".into(),
        };
        assert_eq!(
            root_home(&top, None).unwrap(),
            ("share://room/r1/d1".into(), None)
        );
        let drive = WorkRoot {
            spec: "gdrive:///Docs".into(),
            kind: RootKind::Drive,
            path: "/Docs".into(),
        };
        let refused = root_home(&drive, None).unwrap_err();
        assert!(refused.contains("give `target`"), "{refused}");
        assert!(remote_relative("src"));
        for absolute in ["/srv", "~/x", "C:\\x", "$HOME/x", "%TEMP%", "\\\\host\\s"] {
            assert!(!remote_relative(absolute), "{absolute}");
        }
    }
}
