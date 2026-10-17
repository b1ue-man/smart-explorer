//! The exec streams a session runs on: the SSH exec channel of a saved SFTP
//! remote (`sftp/core/command.rs`) or a Share peer's contained Exec stream
//! through the background worker (`daemon::connect_exec`). Both carry stdin,
//! stdout and stderr as bytes and report how the remote process ended. The
//! same selectors as `se exec` choose the target. The terminal variants of
//! both (a login shell in a PTY) live in `terminal_transports.rs` and reuse
//! the event mapping here.
//!
//! A Share start can wait for the other device's user (PRM-M3): until it
//! begins, its notice is read from the worker, the status line goes to
//! stderr, and the start timers do not count the wait. The session's shell
//! start names the account it runs as (`{device}: as {account}`).
use super::dialect;
use super::terminal_transports::{print_account_line, PendingNotice};
use super::types::{Dialect, TerminalSize};
use crate::creds::SavedConnection;
use crate::daemon::{ExecIpcEvent, ExecIpcInput, ExecIpcSession};
use crate::sftp::{SftpBackend, SshCommand, SshCommandControl, SshCommandEvent};
use crate::share::{ExecCommand, ExecTerminal, ExecTerminalKind, PeerOpenTarget};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

/// Longest wait for the login-shell probe; it only echoes.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Probe output kept for the dialect decision.
const MAX_PROBE_BYTES: usize = 64 * 1024;

pub(super) enum TransportEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// Always the last event.
    Ended(TransportEnd),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TransportEnd {
    /// Exit status of the remote shell, `128 + n` after signal `n`.
    pub(super) exit_code: Option<i32>,
    pub(super) cancelled: bool,
    pub(super) error: Option<String>,
}

pub(super) trait TransportInput: Send + Sync {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String>;
    fn eof(&self) -> Result<(), String>;
    /// Ends the remote process tree (SSH: TERM, then KILL; Share: contained tree).
    fn cancel(&self);
    /// Gives the remote terminal a new size. Only transports of a terminal
    /// session have one.
    fn resize(&self, _size: TerminalSize) -> Result<(), String> {
        Err("no terminal".to_string())
    }

    /// The start waits for the other device's user (Share only).
    fn waiting_for_approval(&self) -> bool {
        false
    }
}

pub(super) trait Transport: Send {
    /// Blocks until the next event. Must not run inside a tokio runtime.
    fn next_event(&mut self) -> TransportEvent;
    fn input(&self) -> Arc<dyn TransportInput>;
}

/// Where a session runs.
pub(in crate::cli) enum TargetSpec {
    Ssh {
        connection: SavedConnection,
        label: String,
        cwd: Option<String>,
    },
    Share {
        peer: PeerOpenTarget,
        label: String,
    },
}

impl TargetSpec {
    /// The selector as given (`@label`, endpoint).
    pub(in crate::cli) fn label(&self) -> &str {
        match self {
            Self::Ssh { label, .. } | Self::Share { label, .. } => label,
        }
    }

    /// Identity of the remote: equal for every selector of the same saved
    /// connection or peer.
    pub(in crate::cli) fn key(&self) -> String {
        match self {
            Self::Ssh { connection, .. } => format!("ssh {}", connection.account()),
            Self::Share { peer, .. } => format!("share {}", peer.endpoint_prefix()),
        }
    }

    /// Working directory named by the selector (`@label:/dir`).
    pub(super) fn cwd(&self) -> Option<&str> {
        match self {
            Self::Ssh { cwd, .. } => cwd.as_deref(),
            Self::Share { .. } => None,
        }
    }
}

/// A saved SFTP remote (`@label[:/dir]`, its `sftp://` endpoint) or a Share
/// peer (name, id, `share://` endpoint; omitted when exactly one is ready).
pub(in crate::cli) fn resolve_target(selector: Option<&str>) -> Result<TargetSpec, String> {
    let selector = selector.map(str::trim).filter(|value| !value.is_empty());
    if let Some(given) = selector {
        if let Some(target) = crate::cli::exec_ssh::ssh_target(given)? {
            return Ok(TargetSpec::Ssh {
                connection: target.connection,
                label: given.to_string(),
                cwd: target.cwd,
            });
        }
    }
    let peer = crate::cli::exec::select_target(selector)?;
    let label = selector
        .map(str::to_string)
        .unwrap_or_else(|| peer.endpoint_prefix());
    Ok(TargetSpec::Share { peer, label })
}

/// Recognizes the login shell, then starts the session's shell on `target`.
pub(super) fn connect(
    target: &TargetSpec,
    ssh: &dyn Fn(&SavedConnection) -> Result<Arc<SftpBackend>, String>,
) -> Result<(Dialect, Box<dyn Transport>), String> {
    match target {
        TargetSpec::Ssh { connection, .. } => {
            let backend = ssh(connection)?;
            let probe = run_probe(Box::new(ssh_start(&backend, dialect::PROBE)?))?;
            let dialect = dialect::classify(&probe);
            let shell = ssh_start(&backend, dialect::start_line(dialect))?;
            Ok((dialect, Box::new(shell)))
        }
        TargetSpec::Share { peer, label } => {
            let probe = share_start(peer, dialect::PROBE, Some(PROBE_TIMEOUT), label, false)?;
            let dialect = dialect::classify(&run_probe(Box::new(probe))?);
            let shell = share_start(peer, dialect::start_line(dialect), None, label, true)?;
            Ok((dialect, Box::new(shell)))
        }
    }
}

pub(super) fn run_probe(mut transport: Box<dyn Transport>) -> Result<String, String> {
    let input = transport.input();
    let _ = input.eof();
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let timer = input.clone();
    std::thread::Builder::new()
        .name("se-shell-probe".into())
        .spawn(move || loop {
            match finished.recv_timeout(PROBE_TIMEOUT) {
                // The other device's user deciding does not count.
                Err(RecvTimeoutError::Timeout) if timer.waiting_for_approval() => {}
                Ok(()) => return,
                Err(_) => {
                    timer.cancel();
                    return;
                }
            }
        })
        .map_err(|error| format!("cannot start the probe timer: {error}"))?;
    let mut stdout = Vec::new();
    let end = loop {
        match transport.next_event() {
            TransportEvent::Stdout(bytes) => {
                let room = MAX_PROBE_BYTES.saturating_sub(stdout.len());
                stdout.extend_from_slice(&bytes[..bytes.len().min(room)]);
            }
            TransportEvent::Stderr(_) => {}
            TransportEvent::Ended(end) => break end,
        }
    };
    let _ = done.send(());
    if end.cancelled {
        return Err(format!(
            "the remote login shell did not answer within {} seconds",
            PROBE_TIMEOUT.as_secs()
        ));
    }
    match end.error {
        Some(error) if stdout.is_empty() => Err(format!(
            "the remote login shell could not run commands: {error}"
        )),
        _ => Ok(String::from_utf8_lossy(&stdout).into_owned()),
    }
}

struct SshInput(SshCommandControl);

impl TransportInput for SshInput {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.stdin(bytes).map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        self.0.eof().map_err(|error| error.to_string())
    }

    fn cancel(&self) {
        self.0.cancel();
    }
}

pub(super) struct SshTransport {
    command: SshCommand,
    input: Arc<SshInput>,
}

pub(super) fn ssh_start(backend: &Arc<SftpBackend>, line: &str) -> Result<SshTransport, String> {
    let command = backend
        .clone()
        .start_command(line)
        .map_err(|error| format!("remote shell start: {error}"))?;
    let input = Arc::new(SshInput(command.control()));
    Ok(SshTransport { command, input })
}

impl Transport for SshTransport {
    fn next_event(&mut self) -> TransportEvent {
        next_ssh_event(&mut self.command)
    }

    fn input(&self) -> Arc<dyn TransportInput> {
        self.input.clone()
    }
}

/// The next event of an SSH channel as a session event; blocks.
pub(super) fn next_ssh_event(command: &mut SshCommand) -> TransportEvent {
    loop {
        match command.next_event() {
            Some(SshCommandEvent::Started) => {}
            Some(SshCommandEvent::Stdout(bytes)) => return TransportEvent::Stdout(bytes),
            Some(SshCommandEvent::Stderr(bytes)) => return TransportEvent::Stderr(bytes),
            Some(SshCommandEvent::Finished(end)) => {
                let signalled = end
                    .signal
                    .as_deref()
                    .and_then(crate::sftp::signal_number)
                    .map(|number| 128 + number);
                return TransportEvent::Ended(TransportEnd {
                    exit_code: end
                        .exit_status
                        .map(|status| i32::try_from(status).unwrap_or(255))
                        .or(signalled),
                    cancelled: end.cancelled,
                    error: end.error,
                });
            }
            None => {
                return TransportEvent::Ended(TransportEnd {
                    error: Some("the SSH channel ended without a final status".into()),
                    ..TransportEnd::default()
                })
            }
        }
    }
}

struct ShareInput {
    input: ExecIpcInput,
    notice: PendingNotice,
}

impl TransportInput for ShareInput {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        self.input.stdin(bytes).map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        self.input.eof().map_err(|error| error.to_string())
    }

    fn cancel(&self) {
        let _ = self.input.cancel();
    }

    fn waiting_for_approval(&self) -> bool {
        self.notice.waiting()
    }
}

struct ShareTransport {
    session: ExecIpcSession,
    input: Arc<ShareInput>,
    /// The peer as the user selected it.
    label: String,
    /// The session's account line is still to be shown.
    header: bool,
}

fn share_start(
    peer: &PeerOpenTarget,
    line: &str,
    timeout: Option<Duration>,
    label: &str,
    header: bool,
) -> Result<ShareTransport, String> {
    let start = crate::share::ExecStart {
        exec_id: crate::share::ExecId::generate().map_err(|error| error.to_string())?,
        command: ExecCommand::Shell {
            command: line.to_string(),
        },
        cwd: None,
        env: std::collections::BTreeMap::new(),
        timeout_ms: timeout.map(|limit| u64::try_from(limit.as_millis()).unwrap_or(u64::MAX)),
        max_output_bytes: None,
        terminal: None,
    };
    start.validate().map_err(|error| error.to_string())?;
    let session = crate::daemon::connect_exec(peer.clone(), start)?;
    let input = Arc::new(ShareInput {
        input: session.take_input().map_err(|error| error.to_string())?,
        notice: PendingNotice::follow(&session.exec_id, label),
    });
    Ok(ShareTransport {
        session,
        input,
        label: label.to_string(),
        header,
    })
}

impl Transport for ShareTransport {
    fn next_event(&mut self) -> TransportEvent {
        let Self {
            session,
            input,
            label,
            header,
        } = self;
        let pending_header = header.then(|| session.exec_id.clone());
        next_share_event_seen(session, &mut |event| match event {
            ExecIpcEvent::Authorized(provider) => {
                if let Some(exec_id) = &pending_header {
                    *header = false;
                    print_account_line(exec_id, label, &provider.user_label);
                }
            }
            _ => input.notice.stop(),
        })
    }

    fn input(&self) -> Arc<dyn TransportInput> {
        self.input.clone()
    }
}

/// The next event of a Share Exec stream as a session event; blocks.
pub(super) fn next_share_event(session: &mut ExecIpcSession) -> TransportEvent {
    next_share_event_seen(session, &mut |_| {})
}

/// `next_share_event`; `seen` sees every Exec event first.
pub(super) fn next_share_event_seen(
    session: &mut ExecIpcSession,
    seen: &mut dyn FnMut(&ExecIpcEvent),
) -> TransportEvent {
    loop {
        let event = session.next_event();
        if let Ok(event) = &event {
            seen(event);
        }
        match event {
            Ok(ExecIpcEvent::Authorized(_) | ExecIpcEvent::Started) => {}
            Ok(ExecIpcEvent::Stdout(bytes)) => return TransportEvent::Stdout(bytes),
            Ok(ExecIpcEvent::Stderr(bytes)) => return TransportEvent::Stderr(bytes),
            Ok(ExecIpcEvent::Terminal(terminal)) => {
                return TransportEvent::Ended(terminal_end(&terminal))
            }
            Ok(ExecIpcEvent::Failed(failure)) => {
                return TransportEvent::Ended(TransportEnd {
                    error: Some(format!("{}: {}", failure.code, failure.message)),
                    ..TransportEnd::default()
                })
            }
            Err(error) => {
                return TransportEvent::Ended(TransportEnd {
                    error: Some(format!("remote execution transport: {error}")),
                    ..TransportEnd::default()
                })
            }
        }
    }
}

fn terminal_end(terminal: &ExecTerminal) -> TransportEnd {
    match terminal.kind {
        ExecTerminalKind::Exited => TransportEnd {
            exit_code: terminal
                .exit_code
                .or_else(|| terminal.signal.map(|signal| 128 + signal)),
            ..TransportEnd::default()
        },
        ExecTerminalKind::Cancelled => TransportEnd {
            cancelled: true,
            ..TransportEnd::default()
        },
        ExecTerminalKind::TimedOut => TransportEnd {
            cancelled: true,
            error: Some("the remote shell reached its time limit".into()),
            ..TransportEnd::default()
        },
        ExecTerminalKind::Failed | ExecTerminalKind::Revoked | ExecTerminalKind::Disconnected => {
            TransportEnd {
                error: Some(terminal.message.clone().unwrap_or_else(|| {
                    format!("the remote shell ended: {:?}", terminal.kind).to_ascii_lowercase()
                })),
                ..TransportEnd::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{terminal_end, TransportEnd};
    use crate::share::{ExecId, ExecTerminal, ExecTerminalKind};

    #[test]
    fn shell_session_task_share_terminals_keep_exit_codes() {
        let mut terminal = ExecTerminal {
            exec_id: ExecId::parse("00".repeat(16)).unwrap(),
            kind: ExecTerminalKind::Exited,
            exit_code: Some(4),
            signal: None,
            message: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            output_truncated: false,
        };
        assert_eq!(terminal_end(&terminal).exit_code, Some(4));
        terminal.exit_code = None;
        terminal.signal = Some(9);
        assert_eq!(terminal_end(&terminal).exit_code, Some(137));
        terminal.kind = ExecTerminalKind::Cancelled;
        assert_eq!(
            terminal_end(&terminal),
            TransportEnd {
                cancelled: true,
                ..TransportEnd::default()
            }
        );
        terminal.kind = ExecTerminalKind::Revoked;
        terminal.message = Some("grant removed".into());
        assert_eq!(
            terminal_end(&terminal).error.as_deref(),
            Some("grant removed")
        );
    }
}
