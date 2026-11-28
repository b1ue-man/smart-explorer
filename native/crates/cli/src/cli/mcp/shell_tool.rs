//! `shell`: commands through Smart Explorer instead of the agent's own shell
//! (plan docs/plaene/2026-11-28-ersetzen-masken-shell, M6; part C row
//! „Shell-Werkzeug“). Without a working root the command runs on this
//! device; with one it runs where the root is, in the root's folder (a
//! relative `cwd` lies below it):
//!
//! - a local folder (or, on Windows, a UNC folder): this device;
//! - an SSH remote: over its SSH login, as `exec` does;
//! - a Share peer: on that device, which turns the root's Share folder into
//!   its disk folder (`share_cwd`);
//! - FTP, WebDAV, SMB logins, Google Drive and vaults hold only files: a
//!   clear error.
//!
//! Commands run with se's rights; `elevated` asks the user for
//! administrator or root rights (UAC, polkit, `sudo` on SSH remotes), one
//! dialog per command or per elevated `session`. Placeholders of the mask
//! lists in the arguments are restored before the tool runs and the output
//! goes through the lists (the MCP layer, contract 4); `shell_tool_guard.rs`
//! refuses se's own administration and, while a list is active, guarded
//! places, and checks the mask store after every command on this device.
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use super::args::Args;
use super::exec_session_job::LocalOpening;
use super::exec_tool::{self, ExecRequest, RemoteHome};
use super::jobs::Job;
use super::root_path::{RootKind, WorkRoot};
use super::session::Session;
use super::shell_tool_guard::{self as guard, CallTexts};
use super::shell_tool_local::LocalRun;
use super::shell_tool_shells::{self as shells, LocalHost};
use crate::cli::shell_session::TargetSpec;
use crate::creds::Protocol;
use crate::elevate::ElevatedSpec;

pub(super) const DESCRIPTION: &str = "Runs a command through Smart Explorer instead of your own terminal. Without a working root it runs on this device (Linux: the login shell from $SHELL, else /bin/sh; Windows: PowerShell 7 when installed, else Windows PowerShell 5.1, which has no `&&`/`||`: write `a; if ($?) { b }`; `shell` picks cmd or sh). With a working root (`root`) it runs where the root is, in the root's folder (a relative `cwd` is below it): a local folder on this device, an SSH remote over its SSH login (remote login shell), a Share peer on that device (in the root's Share folder; PowerShell on Windows peers). FTP, WebDAV, SMB, Google Drive and vault roots have no shell. Commands run with Smart Explorer's rights; `elevated: true` asks the user for administrator or root rights (UAC on Windows, polkit on Linux, sudo on SSH remotes with a locally asked password, the peer's own dialog on Share peers): one dialog per command, one per elevated `session`. While the dialog is open the job's progress says `waiting_for_elevation`. Mask list placeholders in `command`, `argv`, `cwd`, `env` and `stdin` are restored before the command runs; its output comes back masked. Commands that run se's own mask administration (`se masks`, `se elevated`, `se hook`) are refused; while a mask list guards a folder, so are commands that name se's own data or the agent hosts' configuration. The result has exit_code, signal, cli_exit_code (124 timeout, 130 cancelled, 126 elevation declined, 125 not started), `shell` and `elevation`. stdout/stderr are kept completely and paged; job_input sends input, job_cancel ends the whole process tree. With `session` (a name) the command runs in a persistent shell that keeps its working directory, environment and state (see `exec`); `sessions` lists them.";

const WAIT: &str = "Seconds to wait for completion before returning a job_id (default 30). Keep it below your client's tool timeout (Codex: 60 s by default).";
const MAX_BYTES: &str = "Payload budget of this answer in bytes (default 20000). Agent hosts cut results above about 10000 tokens.";

pub(super) fn properties() -> Value {
    json!({
        "command": {"type": "string", "description": "One command string for the shell of the place it runs: PowerShell on Windows (pwsh 7, else 5.1 without && and ||), the login shell on Linux and SSH remotes."},
        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Program and literal arguments, instead of `command` (no shell)."},
        "shell": {"type": "string", "enum": ["powershell", "cmd", "sh"], "description": "Shell for `command` instead of the default: powershell or cmd on Windows (sh where an sh.exe is installed), sh on Linux and SSH remotes."},
        "cwd": {"type": "string", "description": "Working directory: relative to the working root's folder, or absolute on the device that runs the command (default: the root's folder; without a root this server's working directory)."},
        "env": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Extra environment variables."},
        "stdin": {"type": "string", "description": "Text written to standard input, followed by end-of-input unless `keep_stdin_open`."},
        "keep_stdin_open": {"type": "boolean", "description": "Leave standard input open for job_input."},
        "timeout_seconds": {"type": "integer", "minimum": 1, "description": "End the command after this many seconds (default: no limit); in a session this ends the session."},
        "session": {"type": "string", "description": "Persistent shell session name (letters, digits, '-', '_', '.'): opened at the place of the command by the first call, reused by later ones."},
        "attachable": {"type": "boolean", "description": "Only when the call opens a session: let the user's `se shell --attach` and other agents join it (default true)."},
        "elevated": {"type": "boolean", "description": "Run with administrator or root rights; the user confirms in the system dialog (default false)."},
        "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT},
        "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
    })
}

/// Where the command runs.
#[derive(Debug, PartialEq, Eq)]
enum Place {
    /// This device, in this folder (`None`: the server's own).
    Local { cwd: Option<String> },
    /// The root's SSH remote or Share peer.
    Remote(RemoteHome),
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    if let Some(reason) = guard::session_lock() {
        return Err(reason);
    }
    let command = exec_tool::command_of(args)?;
    let shell = exec_tool::shell_of(args, &command)?;
    let session_name = exec_tool::session_of(args)?;
    let cwd = args.opt_string("cwd")?.filter(|cwd| !cwd.is_empty());
    let env = args.string_map("env")?;
    let stdin = args.opt_string("stdin")?;
    let elevated = args.flag("elevated")?;
    let attachable = exec_tool::attachable_of(args)?;
    let explicit_cwd = cwd.is_some();
    let set = session.masks.set()?;
    let place = place(session.targets.root().as_deref(), cwd)?;
    let local_cwd = match &place {
        Place::Local { cwd } => cwd.clone(),
        Place::Remote(_) => None,
    };
    let texts = CallTexts {
        command: &command,
        cwd: local_cwd.as_deref(),
        env: &env,
        stdin: stdin.as_deref(),
    };
    if let Some(reason) = guard::refusal(&set, &texts) {
        return Err(reason);
    }
    let mut request = ExecRequest {
        selector: String::new(),
        command,
        cwd: None,
        env,
        stdin,
        keep_stdin_open: args.flag("keep_stdin_open")?,
        timeout_seconds: args
            .opt_u64("timeout_seconds")?
            .filter(|seconds| *seconds > 0),
        shell,
        share_cwd: None,
        elevated,
    };
    match place {
        Place::Remote(home) => {
            request.selector = home.selector;
            request.cwd = home.cwd;
            request.share_cwd = home.share_cwd;
            exec_tool::run_remote(session, request, session_name, attachable)
        }
        Place::Local { cwd } => {
            let host = LocalHost::detect();
            let guard = guard::take_guard(&session.masks, &set)?;
            match session_name {
                Some(name) => {
                    let opening = SessionPlace {
                        name,
                        attachable,
                        cwd,
                        explicit_cwd,
                    };
                    local_session(session, request, opening, &host, guard)
                }
                None => local_command(session, request, cwd, &host, guard),
            }
        }
    }
}

fn local_command(
    session: &Session,
    request: ExecRequest,
    cwd: Option<String>,
    host: &LocalHost,
    guard: Option<crate::masks::IntegrityGuard>,
) -> Result<Arc<Job>, String> {
    let (command, shell) = shells::one_shot(host, request.shell, &request.command)?;
    let summary = shorten(format!(
        "shell{}: {}",
        if request.elevated { " (elevated)" } else { "" },
        request.command_text()
    ));
    super::shell_tool_local::start(
        session,
        LocalRun {
            spec: ElevatedSpec {
                command,
                cwd,
                env: request.env,
            },
            shell,
            elevated: request.elevated,
            stdin: request.stdin,
            keep_stdin_open: request.keep_stdin_open,
            timeout_seconds: request.timeout_seconds,
            summary,
        },
        guard,
    )
}

/// A session on this device: its name, whether others may join, the
/// folder it opens in, and whether the call named that folder itself.
struct SessionPlace {
    name: String,
    attachable: bool,
    cwd: Option<String>,
    explicit_cwd: bool,
}

fn local_session(
    session: &Session,
    mut request: ExecRequest,
    place: SessionPlace,
    host: &LocalHost,
    guard: Option<crate::masks::IntegrityGuard>,
) -> Result<Arc<Job>, String> {
    let (command, dialect) = shells::session_shell(host, request.shell)?;
    // The session's shell is this one; `shell` was used to choose it.
    request.shell = None;
    let label = if request.elevated {
        "this device (elevated)"
    } else {
        "this device"
    };
    request.selector = label.to_string();
    let spec = TargetSpec::Local {
        label: label.to_string(),
        start: ElevatedSpec {
            command,
            cwd: None,
            env: Default::default(),
        },
        dialect,
        elevated: request.elevated,
    };
    // A new session starts in the folder; a command of an open one changes
    // to a folder only when the call named it.
    request.cwd = place.cwd.clone().filter(|_| place.explicit_cwd);
    let job = super::exec_session_job::start_with(
        session,
        request,
        place.name,
        place.attachable,
        Some(LocalOpening {
            spec,
            cwd: place.cwd,
        }),
    )?;
    watch_store(job.clone(), guard);
    Ok(job)
}

/// The store check after a session command: on its own thread, once the
/// job ended.
fn watch_store(job: Arc<Job>, guard: Option<crate::masks::IntegrityGuard>) {
    let Some(mut guard) = guard else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name(format!("se-mcp-{}-store", job.id))
        .spawn(move || {
            while job.end().is_none() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = guard::check_after(Some(&mut guard));
        });
}

/// Where a command runs for the working root `root`.
fn place(root: Option<&WorkRoot>, cwd: Option<String>) -> Result<Place, String> {
    let Some(root) = root else {
        return Ok(Place::Local { cwd });
    };
    match root.kind {
        RootKind::Local => local_place(root, cwd),
        RootKind::Unc if cfg!(windows) => local_place(root, cwd),
        RootKind::Login(Protocol::Sftp) | RootKind::Peer => {
            exec_tool::root_home(root, cwd).map(Place::Remote)
        }
        kind => Err(format!(
            "the working root {} is a {} location, which holds files but has no shell; `shell` runs on this device, on SSH remotes and on Share peers: clear the root (`root` with action clear) to run on this device, or set a root there",
            root.spec,
            kind.label(&root.spec)
        )),
    }
}

/// A local root: its folder, a relative `cwd` below it (never above).
fn local_place(root: &WorkRoot, cwd: Option<String>) -> Result<Place, String> {
    let base = if root.path.is_empty() {
        root.spec.clone()
    } else {
        root.path.clone()
    };
    let cwd = match cwd {
        Some(cwd) if super::root_path::is_absolute_local(&cwd) => cwd,
        Some(cwd) => {
            let rel = super::root_path::normalize_relative(&cwd, &root.spec)?;
            rel.split('/')
                .filter(|part| !part.is_empty())
                .fold(Path::new(&base).to_path_buf(), |path, part| path.join(part))
                .to_string_lossy()
                .into_owned()
        }
        None => base,
    };
    Ok(Place::Local { cwd: Some(cwd) })
}

fn shorten(mut text: String) -> String {
    if text.chars().count() > 160 {
        text = text.chars().take(159).chain(std::iter::once('…')).collect();
    }
    text
}

#[cfg(test)]
pub(super) fn place_for_tests(
    root: Option<&WorkRoot>,
    cwd: Option<String>,
) -> Result<(Option<String>, Option<RemoteHome>), String> {
    place(root, cwd).map(|place| match place {
        Place::Local { cwd } => (cwd, None),
        Place::Remote(home) => (None, Some(home)),
    })
}
