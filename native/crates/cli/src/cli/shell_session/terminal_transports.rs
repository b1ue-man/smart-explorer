//! The transports of terminal sessions: the login shell of an SSH server
//! (a session channel with `pty-req`, then `shell`, or `exec` when a working
//! directory or environment must be set, `sftp/core/terminal.rs`) or of a
//! Share peer (an Exec start of `ExecCommand::Login` with a terminal request)
//! in a pseudo-terminal. Both carry the terminal's bytes unchanged and take
//! size changes; the event mapping is that of the command transports. A
//! Share terminal that waits for the other device's user (PRM-M3) shows the
//! status line and does not count the wait against its start time; once it
//! runs, its account line goes to stderr before the terminal takes over. The
//! notice helpers and the dialect question inside a running Share session
//! (`probe_in_session`) serve the command transports as well.
use super::dialect;
use super::transports::{
    self, next_share_event, next_ssh_event, TargetSpec, Transport, TransportEvent, TransportInput,
    PROBE_TIMEOUT,
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
/// Output of a session's shell kept while its dialect is asked.
const MAX_SESSION_PROBE_BYTES: usize = 64 * 1024;

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

/// Asks the shell of a running Share session which dialect it speaks
/// (`transports::share_start_line` started PowerShell or a POSIX shell): it
/// answers `echo SE_DIALECT_<nonce>_$?` with `True`/`False` (PowerShell) or
/// an exit status (POSIX). Output before the answer is login noise. The wait
/// for the other device's user does not count against the 30 seconds.
pub(super) fn probe_in_session(transport: &mut dyn Transport) -> Result<Dialect, String> {
    let nonce = ExecId::generate().map_err(|error| error.to_string())?;
    let marker = format!("SE_DIALECT_{nonce}_");
    let input = transport.input();
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let timer = input.clone();
    std::thread::Builder::new()
        .name("se-shell-dialect".into())
        .spawn(move || loop {
            match finished.recv_timeout(PROBE_TIMEOUT) {
                Err(RecvTimeoutError::Timeout) if timer.waiting_for_approval() => {}
                Ok(()) => return,
                Err(_) => {
                    timer.cancel();
                    return;
                }
            }
        })
        .map_err(|error| format!("cannot start the probe timer: {error}"))?;
    let asked = input.stdin(format!("echo {marker}$?\n").as_bytes());
    let mut stdout = Vec::new();
    let answer = loop {
        if let Err(error) = &asked {
            break Err(format!("the remote shell takes no input: {error}"));
        }
        match transport.next_event() {
            TransportEvent::Stdout(bytes) => {
                stdout.extend_from_slice(&bytes);
                let excess = stdout.len().saturating_sub(MAX_SESSION_PROBE_BYTES);
                stdout.drain(..excess);
                if let Some(dialect) = session_dialect(&String::from_utf8_lossy(&stdout), &marker) {
                    break Ok(dialect);
                }
            }
            TransportEvent::Stderr(_) => {}
            TransportEvent::Ended(end) if end.cancelled => {
                break Err(format!(
                    "the remote shell did not answer within {} seconds",
                    PROBE_TIMEOUT.as_secs()
                ))
            }
            TransportEvent::Ended(end) => {
                let why = end.error.unwrap_or_else(|| "it ended".to_string());
                break Err(format!("the remote shell could not run commands: {why}"));
            }
        }
    };
    let _ = done.send(());
    if answer.is_err() {
        input.cancel();
    }
    answer
}

/// The dialect once the answer line to `marker` is complete: PowerShell
/// answers `True` or `False`, a POSIX shell its exit status. A line that
/// repeats the question (`$?`) is an echo of the input.
pub(super) fn session_dialect(output: &str, marker: &str) -> Option<Dialect> {
    let mut rest = output;
    while let Some(at) = rest.find(marker) {
        let after = &rest[at + marker.len()..];
        let end = after.find('\n')?;
        let value = after[..end].trim();
        if value != "$?" {
            let powershell =
                value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false");
            return Some(if powershell {
                Dialect::PowerShell
            } else {
                Dialect::Posix
            });
        }
        rest = &after[end..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::session_dialect;
    use crate::cli::shell_session::transports::share_start_line;
    use crate::cli::shell_session::types::Dialect;

    const MARKER: &str = "SE_DIALECT_00ff_";

    #[test]
    fn shell_session_task_share_session_dialect_is_asked_inside_the_shell() {
        let posix = format!("motd\n{MARKER}0\n");
        assert_eq!(session_dialect(&posix, MARKER), Some(Dialect::Posix));
        let powershell = format!("PS C:\\Users\\a> {MARKER}True\r\nPS C:\\Users\\a> ");
        assert_eq!(
            session_dialect(&powershell, MARKER),
            Some(Dialect::PowerShell)
        );
        let echoed = format!("echo {MARKER}$?\n{MARKER}False\r\n");
        assert_eq!(session_dialect(&echoed, MARKER), Some(Dialect::PowerShell));
        // The answer line is not complete yet.
        assert_eq!(session_dialect(&format!("{MARKER}Tr"), MARKER), None);
        assert_eq!(session_dialect("motd only\n", MARKER), None);
    }

    #[test]
    fn shell_session_task_share_session_needs_one_start_line() {
        let line = share_start_line();
        // cmd.exe: a failing first command with its error sent to NUL, then
        // PowerShell on the session's input, then `rem` for the rest.
        let (cmd, posix) = line.split_once("'; ").unwrap();
        assert_eq!(
            cmd,
            "SE_PS=' 2>NUL & powershell.exe -NoLogo -NoProfile -NonInteractive -Command - & rem "
        );
        // POSIX: the single-quoted part is one assignment; the session shell
        // follows unchanged.
        assert!(!cmd["SE_PS='".len()..].contains('\''));
        assert!(!line.contains(['"', '%', '^', '!']));
        assert_eq!(
            posix,
            "exec sh -c 'if command -v bash >/dev/null 2>&1; then exec bash -l; fi; exec sh -l'"
        );
    }
}
