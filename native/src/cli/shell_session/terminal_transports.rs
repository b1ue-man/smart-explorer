//! The transports of terminal sessions: the login shell of an SSH server
//! (a session channel with `pty-req`, then `shell`, or `exec` when a working
//! directory or environment must be set, `sftp/core/terminal.rs`) or of a
//! Share peer (an Exec start of `ExecCommand::Login` with a terminal request)
//! in a pseudo-terminal. Both carry the terminal's bytes unchanged and take
//! size changes; the event mapping is that of the command transports. A
//! Share terminal that waits for the other device's user (PRM-M3) shows the
//! status line and does not count the wait against its start time; once it
//! runs, its account line goes to stderr before the terminal takes over. The
//! notice helpers at the end serve the command transports as well.
use super::dialect;
use super::transports::{
    self, next_share_event, next_ssh_event, TargetSpec, Transport, TransportEvent, TransportInput,
};
use super::types::{Dialect, TerminalOptions, TerminalSize};
use crate::creds::SavedConnection;
use crate::daemon::{ExecIpcEvent, ExecIpcInput, ExecIpcSession, ExecNoticeView, ExecNoticeWatch};
use crate::sftp::{
    SftpBackend, SshCommand, SshCommandControl, SshTerminalError, SshTerminalRequest,
};
use crate::share::exec_client_messages::{account_line, approval_status_line};
use crate::share::{ExecCommand, ExecId, ExecStart, ExecTerminalRequest, PeerOpenTarget};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Longest wait for a Share peer's shell to start; the connection setup alone
/// may take up to 45 s.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// `ExecIpcFailure::code` of a peer without terminal support.
const TERMINAL_UNSUPPORTED: &str = "terminal_unsupported";

pub(super) enum TerminalConnect {
    /// The terminal runs. The dialect is known only when the target was probed
    /// (SSH with a working directory or environment).
    Ready(Box<dyn Transport>, Option<Dialect>),
    /// The target cannot give a terminal; the caller may fall back to a
    /// command session.
    Unsupported(String),
}

/// Starts the login shell of `target` in a terminal of `options.size`.
pub(super) fn connect_terminal(
    target: &TargetSpec,
    ssh: &dyn Fn(&SavedConnection) -> Result<Arc<SftpBackend>, String>,
    options: &TerminalOptions,
) -> Result<TerminalConnect, String> {
    match target {
        TargetSpec::Ssh { connection, .. } => ssh_terminal(&ssh(connection)?, options),
        TargetSpec::Share { peer, label } => share_terminal(peer, label, options),
    }
}

fn ssh_terminal(
    backend: &Arc<SftpBackend>,
    options: &TerminalOptions,
) -> Result<TerminalConnect, String> {
    // Without a working directory and environment the `shell` request starts
    // the login shell as `ssh HOST` does. Otherwise the command line needs the
    // dialect of the login shell, which a probe tells.
    let (command, dialect) = if options.cwd.is_none() && options.env.is_empty() {
        (None, None)
    } else {
        let probe =
            transports::run_probe(Box::new(transports::ssh_start(backend, dialect::PROBE)?))?;
        let dialect = dialect::classify(&probe);
        let line = dialect::terminal_command(dialect, options.cwd.as_deref(), &options.env)?;
        (Some(line), Some(dialect))
    };
    let (cols, rows) = options.size.ssh_dimensions();
    let request = SshTerminalRequest {
        term: options.term.clone(),
        cols,
        rows,
        command,
    };
    match backend.clone().start_terminal(&request) {
        Ok(command) => {
            let transport = SshTerminal {
                input: Arc::new(SshTerminalInput(command.control())),
                command,
            };
            Ok(TerminalConnect::Ready(Box::new(transport), dialect))
        }
        Err(SshTerminalError::Refused) => Ok(TerminalConnect::Unsupported(
            "the SSH server refused a terminal (PTY)".to_string(),
        )),
        Err(SshTerminalError::Failed(error)) => Err(format!("remote terminal start: {error}")),
    }
}

struct SshTerminalInput(SshCommandControl);

impl TransportInput for SshTerminalInput {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.stdin(bytes).map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        self.0.eof().map_err(|error| error.to_string())
    }

    fn cancel(&self) {
        self.0.cancel();
    }

    fn resize(&self, size: TerminalSize) -> Result<(), String> {
        let (cols, rows) = size.ssh_dimensions();
        self.0.resize(cols, rows).map_err(|error| error.to_string())
    }
}

struct SshTerminal {
    command: SshCommand,
    input: Arc<SshTerminalInput>,
}

impl Transport for SshTerminal {
    fn next_event(&mut self) -> TransportEvent {
        next_ssh_event(&mut self.command)
    }

    fn input(&self) -> Arc<dyn TransportInput> {
        self.input.clone()
    }
}

fn share_terminal(
    peer: &PeerOpenTarget,
    label: &str,
    options: &TerminalOptions,
) -> Result<TerminalConnect, String> {
    let start = ExecStart {
        exec_id: ExecId::generate().map_err(|error| error.to_string())?,
        command: ExecCommand::Login,
        cwd: options.cwd.clone(),
        env: options.env.clone(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: Some(ExecTerminalRequest {
            term: options.term.clone(),
            size: options.size.into(),
        }),
    };
    start.validate().map_err(|error| error.to_string())?;
    let mut session = crate::daemon::connect_exec(peer.clone(), start)?;
    let notice = PendingNotice::follow(&session.exec_id, label);
    let input = Arc::new(ShareTerminalInput(
        session.take_input().map_err(|error| error.to_string())?,
    ));
    // A peer that never answers must not hold the caller forever; the other
    // device's user deciding does not count.
    let (started, waiting) = std::sync::mpsc::channel::<()>();
    let (timer, deciding) = (input.clone(), notice.clone());
    std::thread::Builder::new()
        .name("se-term-start".into())
        .spawn(move || loop {
            match waiting.recv_timeout(START_TIMEOUT) {
                Err(RecvTimeoutError::Timeout) if deciding.waiting() => {}
                Err(RecvTimeoutError::Timeout) => {
                    timer.cancel();
                    return;
                }
                _ => return,
            }
        })
        .map_err(|error| format!("cannot start the start timer: {error}"))?;
    let outcome = wait_started(&mut session);
    notice.stop();
    drop(started);
    match outcome {
        Ok(ShellStart::Running { user_label }) => {
            print_account_line(&session.exec_id, label, &user_label);
            Ok(TerminalConnect::Ready(
                Box::new(ShareTerminal { session, input }),
                None,
            ))
        }
        Ok(ShellStart::Unsupported(why)) => Ok(TerminalConnect::Unsupported(why)),
        Err(error) => {
            input.cancel();
            Err(error)
        }
    }
}

/// How the start of a Share terminal ended.
enum ShellStart {
    /// The shell runs; `user_label` is the account hosts up to 0.5.182 name.
    Running { user_label: String },
    /// The peer has no terminal support.
    Unsupported(String),
}

/// Reads the Exec events up to the start of the shell. A peer without
/// terminal support is `Unsupported`; any other failure before the start is
/// an error.
fn wait_started(session: &mut ExecIpcSession) -> Result<ShellStart, String> {
    let mut user_label = String::new();
    loop {
        let event = session
            .next_event()
            .map_err(|error| format!("remote execution transport: {error}"))?;
        match event {
            ExecIpcEvent::Authorized(provider) => user_label = provider.user_label,
            ExecIpcEvent::Started => return Ok(ShellStart::Running { user_label }),
            ExecIpcEvent::Failed(failure) if failure.code == TERMINAL_UNSUPPORTED => {
                return Ok(ShellStart::Unsupported(
                    "the peer runs a Smart Explorer version without terminal support".to_string(),
                ))
            }
            ExecIpcEvent::Failed(failure) => {
                return Err(format!("{}: {}", failure.code, failure.message))
            }
            ExecIpcEvent::Terminal(terminal) => {
                let kind = format!("{:?}", terminal.kind).to_ascii_lowercase();
                return Err(terminal.message.unwrap_or_else(|| {
                    format!("the remote shell ended before it started: {kind}")
                }));
            }
            ExecIpcEvent::Stdout(_) | ExecIpcEvent::Stderr(_) => {
                return Err("the remote shell sent output before it started".to_string())
            }
        }
    }
}

struct ShareTerminalInput(ExecIpcInput);

impl TransportInput for ShareTerminalInput {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.stdin(bytes).map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        self.0.eof().map_err(|error| error.to_string())
    }

    fn cancel(&self) {
        let _ = self.0.cancel();
    }

    fn resize(&self, size: TerminalSize) -> Result<(), String> {
        self.0
            .resize(size.into())
            .map_err(|error| error.to_string())
    }
}

struct ShareTerminal {
    session: ExecIpcSession,
    input: Arc<ShareTerminalInput>,
}

impl Transport for ShareTerminal {
    fn next_event(&mut self) -> TransportEvent {
        next_share_event(&mut self.session)
    }

    fn input(&self) -> Arc<dyn TransportInput> {
        self.input.clone()
    }
}

/// The notice of a Share start until it begins (D.2.2, PRM-M3).
#[derive(Clone)]
pub(super) struct PendingNotice(Arc<Mutex<Option<ExecNoticeWatch>>>);

impl PendingNotice {
    /// Follows the start `exec_id` on the peer the user named `label`; the
    /// status line goes to stderr once per confirmation.
    pub(super) fn follow(exec_id: &ExecId, label: &str) -> Self {
        let label = label.to_string();
        let mut shown = None;
        let watch = ExecNoticeWatch::start(exec_id.clone(), move |notice| {
            let Some(approval) = &notice.approval else {
                return;
            };
            if shown.as_ref() != Some(approval) {
                let device = device_of(notice, &label);
                eprintln!("{}", approval_status_line(&device, approval.surface()));
                shown = Some(approval.clone());
            }
        });
        Self(Arc::new(Mutex::new(Some(watch))))
    }

    pub(super) fn waiting(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(ExecNoticeWatch::waiting)
    }

    /// The start began or ended: no more reading.
    pub(super) fn stop(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
}

/// The peer as the worker names it, else as the user selected it.
fn device_of(notice: &ExecNoticeView, label: &str) -> String {
    if notice.device.trim().is_empty() {
        label.to_string()
    } else {
        notice.device.clone()
    }
}

/// The account line of a Share session on stderr; `user_label` names the
/// account hosts up to 0.5.182 report.
pub(super) fn print_account_line(exec_id: &ExecId, label: &str, user_label: &str) {
    let notice = crate::daemon::exec_notice(exec_id).ok().flatten();
    let device = notice
        .as_ref()
        .map_or_else(|| label.to_string(), |notice| device_of(notice, label));
    let access = notice.as_ref().and_then(|notice| notice.access.as_ref());
    eprintln!("{}", account_line(&device, access, user_label));
}
