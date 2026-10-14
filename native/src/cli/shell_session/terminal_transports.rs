//! The transports of terminal sessions: the login shell of an SSH server
//! (a session channel with `pty-req`, then `shell`, or `exec` when a working
//! directory or environment must be set, `sftp/core/terminal.rs`) or of a
//! Share peer (an Exec start of `ExecCommand::Login` with a terminal request)
//! in a pseudo-terminal. Both carry the terminal's bytes unchanged and take
//! size changes; the event mapping is that of the command transports.
use super::dialect;
use super::transports::{
    self, next_share_event, next_ssh_event, TargetSpec, Transport, TransportEvent, TransportInput,
};
use super::types::{Dialect, TerminalOptions, TerminalSize};
use crate::creds::SavedConnection;
use crate::daemon::{ExecIpcEvent, ExecIpcInput, ExecIpcSession};
use crate::sftp::{
    SftpBackend, SshCommand, SshCommandControl, SshTerminalError, SshTerminalRequest,
};
use crate::share::{ExecCommand, ExecId, ExecStart, ExecTerminalRequest, PeerOpenTarget};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
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
        TargetSpec::Share { peer, .. } => share_terminal(peer, options),
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
    let input = Arc::new(ShareTerminalInput(
        session.take_input().map_err(|error| error.to_string())?,
    ));
    // A peer that never answers must not hold the caller forever.
    let (started, waiting) = std::sync::mpsc::channel::<()>();
    let timer = input.clone();
    std::thread::Builder::new()
        .name("se-term-start".into())
        .spawn(move || {
            if let Err(RecvTimeoutError::Timeout) = waiting.recv_timeout(START_TIMEOUT) {
                timer.cancel();
            }
        })
        .map_err(|error| format!("cannot start the start timer: {error}"))?;
    let outcome = wait_started(&mut session);
    drop(started);
    match outcome {
        Ok(None) => Ok(TerminalConnect::Ready(
            Box::new(ShareTerminal { session, input }),
            None,
        )),
        Ok(Some(why)) => Ok(TerminalConnect::Unsupported(why)),
        Err(error) => {
            input.cancel();
            Err(error)
        }
    }
}

/// Reads the Exec events up to the start of the shell. `Some(why)` is a peer
/// without terminal support; any other failure before the start is an error.
fn wait_started(session: &mut ExecIpcSession) -> Result<Option<String>, String> {
    loop {
        let event = session
            .next_event()
            .map_err(|error| format!("remote execution transport: {error}"))?;
        match event {
            ExecIpcEvent::Authorized(_) => {}
            ExecIpcEvent::Started => return Ok(None),
            ExecIpcEvent::Failed(failure) if failure.code == TERMINAL_UNSUPPORTED => {
                return Ok(Some(
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
