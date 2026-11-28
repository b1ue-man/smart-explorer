//! `exec`: a command on a saved SFTP remote (SSH login) or on a Share peer
//! that granted this device Exec, run as a job with complete stdout/stderr;
//! with `session` in a persistent shell (`exec_session_job.rs`). `elevated`
//! runs it with administrator or root rights: a Share peer's se asks its user
//! (UAC, polkit), an SSH remote runs it through `sudo` (a needed password is
//! asked locally, `exec_ssh_sudo.rs`). Under a Share working root the peer
//! starts in the root's folder itself (`share_cwd`, `exec_host_options.rs`).
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
    /// The working directory as a Share path the peer translates (a Share
    /// working root below the peer's top level); excludes `cwd`.
    pub(super) share_cwd: Option<String>,
    /// With administrator or root rights.
    pub(super) elevated: bool,
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

/// `command` or `argv` of an `exec` or `shell` call.
pub(super) fn command_of(args: &Args<'_>) -> Result<ExecCommand, String> {
    match (args.opt_string("command")?, args.opt_strings("argv")?) {
        (Some(_), Some(_)) => Err("give either `command` or `argv`, not both".into()),
        (Some(command), None) if !command.trim().is_empty() => Ok(ExecCommand::Shell { command }),
        (None, Some(argv)) if !argv.is_empty() => Ok(ExecCommand::Argv {
            program: argv[0].clone(),
            args: argv[1..].to_vec(),
        }),
        _ => Err(
            "missing `command` (one shell string) or `argv` (program and literal arguments)".into(),
        ),
    }
}

/// `shell` of an `exec` or `shell` call, checked against the command.
pub(super) fn shell_of(
    args: &Args<'_>,
    command: &ExecCommand,
) -> Result<Option<ExecShell>, String> {
    let shell = args
        .opt_string("shell")?
        .map(|value| ExecShell::parse(&value))
        .transpose()?;
    if shell.is_some() && !matches!(command, ExecCommand::Shell { .. }) {
        return Err(
            "`shell` chooses the shell of `command`; `argv` starts the program without one".into(),
        );
    }
    Ok(shell)
}

/// The `session` name of an `exec` or `shell` call.
pub(super) fn session_of(args: &Args<'_>) -> Result<Option<String>, String> {
    Ok(args
        .opt_string("session")?
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty()))
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let command = command_of(args)?;
    let shell = shell_of(args, &command)?;
    let session_name = session_of(args)?;
    let explicit = args
        .opt_string("target")?
        .map(|target| target.trim().to_string())
        .filter(|target| !target.is_empty());
    let cwd = args.opt_string("cwd")?.filter(|cwd| !cwd.is_empty());
    // An open session needs no target; opening one does. Without a target
    // the command runs on the working root's SSH remote or Share peer.
    let home = match (explicit, &session_name) {
        (Some(target), _) => RemoteHome::plain(target, cwd),
        (None, Some(name)) if session.shells.is_open(name) => RemoteHome::plain(String::new(), cwd),
        (None, _) => match session.targets.root() {
            // A session keeps its own working directory between commands.
            Some(root) if session_name.is_some() => {
                RemoteHome::plain(super::root_tool::exec_home(&root)?.selector, cwd)
            }
            Some(root) => root_home(&root, cwd)?,
            None if session_name.is_some() => RemoteHome::plain(String::new(), cwd),
            None => {
                return Err(
                    "missing required argument `target` (or set a working root with `root`)".into(),
                )
            }
        },
    };
    let request = ExecRequest {
        selector: home.selector,
        command,
        cwd: home.cwd,
        env: args.string_map("env")?,
        stdin: args.opt_string("stdin")?,
        keep_stdin_open: args.flag("keep_stdin_open")?,
        timeout_seconds: args
            .opt_u64("timeout_seconds")?
            .filter(|seconds| *seconds > 0),
        shell,
        share_cwd: home.share_cwd,
        elevated: args.flag("elevated")?,
    };
    run_remote(session, request, session_name, attachable_of(args)?)
}

/// `attachable` (default true) of a call that may open a session.
pub(super) fn attachable_of(args: &Args<'_>) -> Result<bool, String> {
    match args.value("attachable") {
        None => Ok(true),
        Some(_) => args.flag("attachable"),
    }
}

/// Runs `request` on its SSH remote or Share peer, in the session `name`
/// when one is given.
pub(super) fn run_remote(
    session: &Session,
    request: ExecRequest,
    session_name: Option<String>,
    attachable: bool,
) -> Result<Arc<Job>, String> {
    if let Some(name) = session_name {
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

/// Where a remote command of the working root runs.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct RemoteHome {
    pub(super) selector: String,
    pub(super) cwd: Option<String>,
    pub(super) share_cwd: Option<String>,
}

impl RemoteHome {
    fn plain(selector: String, cwd: Option<String>) -> Self {
        Self {
            selector,
            cwd,
            share_cwd: None,
        }
    }
}

/// The target and working directory for the working root: an SSH root runs
/// in its path (a relative `cwd` below it); a Share root on its peer, in the
/// root's Share folder (a relative `cwd` below it), which the peer turns into
/// its disk folder; an absolute `cwd` is the peer's own path.
pub(super) fn root_home(
    root: &super::root_path::WorkRoot,
    cwd: Option<String>,
) -> Result<RemoteHome, String> {
    let home = super::root_tool::exec_home(root)?;
    if let Some(share_path) = home.share_path {
        return Ok(match cwd {
            Some(cwd) if remote_relative(&cwd) => {
                let rel = super::root_path::normalize_relative(&cwd, &root.spec)?;
                let folder = if rel.is_empty() {
                    share_path
                } else {
                    crate::cli::ops::join(&share_path, &rel)
                };
                RemoteHome {
                    selector: home.selector,
                    cwd: None,
                    share_cwd: Some(folder),
                }
            }
            Some(cwd) => RemoteHome::plain(home.selector, Some(cwd)),
            None => RemoteHome {
                selector: home.selector,
                cwd: None,
                share_cwd: Some(share_path),
            },
        });
    }
    let cwd = match cwd {
        // Relative to the root's folder on the SSH remote.
        Some(cwd) if remote_relative(&cwd) && !root.path.is_empty() => {
            let rel = super::root_path::normalize_relative(&cwd, &root.spec)?;
            Some(crate::cli::ops::join(&root.path, &rel))
        }
        other => other,
    };
    Ok(RemoteHome::plain(home.selector, cwd))
}

/// A working directory on the remote that names no root of its own: not
/// `/…`, `\\…`, `~…`, a drive (`C:…`) or a variable (`$HOME`, `%USERPROFILE%`).
fn remote_relative(cwd: &str) -> bool {
    let drive = cwd.as_bytes().get(1) == Some(&b':');
    !(drive || cwd.starts_with(['/', '\\', '~', '$', '%']))
}

#[cfg(test)]
mod tests {
    use super::{remote_relative, root_home, RemoteHome};
    use crate::cli::mcp::root_path::{RootKind, WorkRoot};
    use crate::creds::Protocol;

    fn home(selector: &str, cwd: Option<&str>, share_cwd: Option<&str>) -> RemoteHome {
        RemoteHome {
            selector: selector.into(),
            cwd: cwd.map(String::from),
            share_cwd: share_cwd.map(String::from),
        }
    }

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
            home("@prod:/srv/app", None, None)
        );
        assert_eq!(
            root_home(&ssh, Some("src/../bin".into()))
                .unwrap()
                .cwd
                .as_deref(),
            Some("/srv/app/bin")
        );
        assert_eq!(
            root_home(&ssh, Some("/tmp".into())).unwrap().cwd.as_deref(),
            Some("/tmp")
        );
        assert!(root_home(&ssh, Some("..".into())).is_err());
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

    #[test]
    fn shell_elevate_task_share_root_starts_in_its_share_folder() {
        let peer = WorkRoot {
            spec: "share://direct/c1/Docs/proj".into(),
            kind: RootKind::Peer,
            path: "/Docs/proj".into(),
        };
        // The peer translates the Share folder (SUCHE-EXEC-SHARE-CWD).
        assert_eq!(
            root_home(&peer, None).unwrap(),
            home("share://direct/c1", None, Some("/Docs/proj"))
        );
        assert_eq!(
            root_home(&peer, Some("src/lib".into())).unwrap(),
            home("share://direct/c1", None, Some("/Docs/proj/src/lib"))
        );
        assert_eq!(
            root_home(&peer, Some(".".into())).unwrap(),
            home("share://direct/c1", None, Some("/Docs/proj"))
        );
        assert!(root_home(&peer, Some("../..".into())).is_err());
        // An absolute `cwd` is the peer's own disk path, as before.
        assert_eq!(
            root_home(&peer, Some("C:\\Users\\a".into())).unwrap(),
            home("share://direct/c1", Some("C:\\Users\\a"), None)
        );
        let top = WorkRoot {
            spec: "share://room/r1/d1/".into(),
            kind: RootKind::Peer,
            path: "/".into(),
        };
        assert_eq!(
            root_home(&top, None).unwrap(),
            home("share://room/r1/d1", None, None)
        );
    }
}
