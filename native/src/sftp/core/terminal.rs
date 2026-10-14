//! Interactive terminal channel on the SSH session of a saved SFTP remote:
//! the transport of `se shell` in a terminal. The channel gets a
//! pseudo-terminal (RFC 4254 6.2 `pty-req`), then the login shell (6.5
//! `shell`) or one command (6.5 `exec`); later size changes go out as 6.7
//! `window-change` (`SshCommandControl::resize`). The remote stderr is part
//! of the terminal, so all output arrives as `SshCommandEvent::Stdout`.
//!
//! Terminal modes sent with every `pty-req` (RFC 4254 section 8, opcode and
//! value): VINTR 3 (^C), VQUIT 28 (^\), VERASE 127 (DEL), VKILL 21 (^U),
//! VEOF 4 (^D), VSUSP 26 (^Z) and IUTF8 1. The server keeps its own default
//! for every other mode.
//!
//! A server that answers the `pty-req` with a failure (`PermitTTY no`,
//! `no-pty` in `authorized_keys`) is reported as
//! [`SshTerminalError::Refused`], so the caller can fall back to command mode.
use super::backend::SftpBackend;
use super::command::{run_channel, SshCommand};
use super::connection::SftpGeneration;
use super::io_err;
use russh::{client, ChannelMsg, Pty};
use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::Duration;

/// Same deadline as the exec request in `exec.rs`; applies to the `pty-req`
/// round trip and again to the `shell` or `exec` request.
const TERMINAL_REQUEST_DEADLINE: Duration = Duration::from_secs(10);
/// Largest columns and rows value (`SHORT` of a Windows console buffer).
const MAX_TERMINAL_DIMENSION: u32 = 32_767;
const MAX_TERM_LEN: usize = 64;

/// Modes of the `pty-req`: control characters of a Linux terminal driver and
/// UTF-8 input handling.
const TERMINAL_MODES: [(Pty, u32); 7] = [
    (Pty::VINTR, 3),
    (Pty::VQUIT, 28),
    (Pty::VERASE, 127),
    (Pty::VKILL, 21),
    (Pty::VEOF, 4),
    (Pty::VSUSP, 26),
    (Pty::IUTF8, 1),
];

/// What `se shell` asks of the remote for one terminal session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshTerminalRequest {
    /// Terminal type for the remote `TERM` (`xterm-256color`): 1 to 64
    /// characters of `A-Z a-z 0-9 . _ + -`.
    pub term: String,
    /// Columns, 1 to 32767.
    pub cols: u32,
    /// Rows, 1 to 32767.
    pub rows: u32,
    /// `None` starts the login shell with the `shell` request, as `ssh HOST`
    /// does. `Some` runs that command with the `exec` request on the
    /// terminal, as `ssh -t HOST command` does.
    pub command: Option<String>,
}

#[derive(Debug)]
pub enum SshTerminalError {
    /// The server rejected the `pty-req`; no program was started and the
    /// channel is closed.
    Refused,
    /// Invalid request values, channel open failure, transport error,
    /// timeout, or a channel closed during the request.
    Failed(io::Error),
}

impl fmt::Display for SshTerminalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused => f.write_str("the SSH server refused a terminal (pty-req)"),
            Self::Failed(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for SshTerminalError {}

impl SshTerminalRequest {
    /// Fail-closed check of the values that go into the `pty-req` (plan E11).
    /// The messages never repeat the rejected `TERM` text.
    fn validate(&self) -> io::Result<()> {
        let term_valid = !self.term.is_empty()
            && self.term.len() <= MAX_TERM_LEN
            && self.term.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-')
            });
        if !term_valid {
            return Err(invalid_request(
                "the terminal type must be 1 to 64 characters of A-Z a-z 0-9 . _ + -",
            ));
        }
        for (name, value) in [("columns", self.cols), ("rows", self.rows)] {
            if !(1..=MAX_TERMINAL_DIMENSION).contains(&value) {
                return Err(invalid_request(&format!(
                    "the terminal {name} must be between 1 and {MAX_TERMINAL_DIMENSION}"
                )));
            }
        }
        Ok(())
    }
}

fn invalid_request(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

/// Server answer to the `pty-req`.
enum PtyAnswer {
    Granted,
    Denied,
}

/// Why the `pty-req` round trip produced no answer.
enum PtyFailure {
    Ssh(russh::Error),
    /// The channel closed (or the connection task ended) before any answer.
    Closed,
}

/// Sends the `pty-req` with a reply and waits for that reply. Other channel
/// messages (`WindowAdjusted`) are skipped: the program is not started yet,
/// so no output can be lost.
async fn negotiate_pty(
    channel: &mut russh::Channel<client::Msg>,
    request: &SshTerminalRequest,
) -> Result<PtyAnswer, PtyFailure> {
    channel
        .request_pty(
            true,
            &request.term,
            request.cols,
            request.rows,
            0,
            0,
            &TERMINAL_MODES,
        )
        .await
        .map_err(PtyFailure::Ssh)?;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(PtyAnswer::Granted),
            Some(ChannelMsg::Failure) => return Ok(PtyAnswer::Denied),
            Some(ChannelMsg::Close) | None => return Err(PtyFailure::Closed),
            Some(_) => {}
        }
    }
}

impl SftpBackend {
    /// Opens a session channel with a PTY and starts the login shell
    /// (`request.command == None`) or `request.command` on it. The returned
    /// handle is driven like [`SshCommand`] from `start_command`: the first
    /// event is `Started` once the server accepted `shell`/`exec`, output
    /// arrives as `Stdout` (stderr is part of the terminal), input goes in
    /// with `stdin`, `resize` sends the window-change, and `Finished` carries
    /// the exit status or signal.
    ///
    /// Must not be called from inside a tokio runtime.
    pub fn start_terminal(
        self: Arc<Self>,
        request: &SshTerminalRequest,
    ) -> Result<SshCommand, SshTerminalError> {
        request.validate().map_err(SshTerminalError::Failed)?;
        let (generation, mut channel) = self
            .open_session_channel()
            .map_err(SshTerminalError::Failed)?;
        let answer = self.rt.block_on(tokio::time::timeout(
            TERMINAL_REQUEST_DEADLINE,
            negotiate_pty(&mut channel, request),
        ));
        match answer {
            Ok(Ok(PtyAnswer::Granted)) => {}
            Ok(Ok(PtyAnswer::Denied)) => {
                let _ = self.rt.block_on(channel.close());
                return Err(SshTerminalError::Refused);
            }
            Ok(Err(PtyFailure::Ssh(error))) => {
                return Err(self.terminal_ssh_failure(&generation, error));
            }
            Ok(Err(PtyFailure::Closed)) => {
                // A channel closed by the server does not prove a dead
                // transport; only a closed session retires the generation.
                if generation.session().is_closed() {
                    self.connection.mark_stale(&generation);
                }
                return Err(SshTerminalError::Failed(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the SSH channel closed during the terminal request",
                )));
            }
            Err(_) => return Err(self.terminal_deadline_failure(&generation)),
        }
        let program = self
            .rt
            .block_on(tokio::time::timeout(TERMINAL_REQUEST_DEADLINE, async {
                match &request.command {
                    Some(command) => channel.exec(true, command.as_bytes().to_vec()).await,
                    None => channel.request_shell(true).await,
                }
            }));
        match program {
            Ok(Ok(())) => Ok(run_channel(self, channel)),
            Ok(Err(error)) => Err(self.terminal_ssh_failure(&generation, error)),
            Err(_) => Err(self.terminal_deadline_failure(&generation)),
        }
    }

    fn terminal_ssh_failure(
        &self,
        generation: &SftpGeneration,
        error: russh::Error,
    ) -> SshTerminalError {
        self.connection.note_ssh_error(generation, &error);
        SshTerminalError::Failed(io_err(error))
    }

    fn terminal_deadline_failure(&self, generation: &SftpGeneration) -> SshTerminalError {
        let error = io::Error::new(io::ErrorKind::TimedOut, "SSH terminal request timed out");
        self.connection.note_io_error(generation, &error);
        SshTerminalError::Failed(error)
    }
}

#[cfg(test)]
mod tests {
    use super::{SshTerminalError, SshTerminalRequest, TERMINAL_MODES};
    use russh::Pty;
    use std::io;

    fn request(term: &str, cols: u32, rows: u32) -> SshTerminalRequest {
        SshTerminalRequest {
            term: term.to_string(),
            cols,
            rows,
            command: None,
        }
    }

    #[test]
    fn terminal_task_modes_are_valid_rfc4254_opcodes() {
        let value_of = |wanted: Pty| {
            TERMINAL_MODES
                .iter()
                .find(|(pty, _)| *pty == wanted)
                .map(|(_, value)| *value)
        };
        assert_eq!(value_of(Pty::VERASE), Some(127));
        assert_eq!(value_of(Pty::IUTF8), Some(1));
        let mut opcodes = Vec::new();
        for (pty, _) in TERMINAL_MODES {
            let opcode = pty as u8;
            // RFC 4254 section 8: opcodes 1..=159; 0 ends the list.
            assert!((1..160).contains(&opcode), "opcode {opcode} is not valid");
            assert!(!opcodes.contains(&opcode), "opcode {opcode} is repeated");
            opcodes.push(opcode);
        }
    }

    #[test]
    fn terminal_task_request_values_are_checked_before_the_channel_opens() {
        assert!(request("xterm-256color", 80, 24).validate().is_ok());
        assert!(request("a", 32_767, 32_767).validate().is_ok());
        for bad in [
            request("", 80, 24),
            request(&"x".repeat(65), 80, 24),
            request("xterm\u{1b}[31m", 80, 24),
            request("xterm 256", 80, 24),
            request("xterm", 0, 24),
            request("xterm", 80, 0),
            request("xterm", 32_768, 24),
            request("xterm", 80, 32_768),
        ] {
            let error = bad.validate().expect_err("request must be rejected");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(!error.to_string().contains('\u{1b}'));
        }
    }

    #[test]
    fn terminal_task_error_text_names_the_refused_pty_request() {
        assert_eq!(
            SshTerminalError::Refused.to_string(),
            "the SSH server refused a terminal (pty-req)"
        );
        let failed = SshTerminalError::Failed(io::Error::other("transport lost"));
        assert_eq!(failed.to_string(), "transport lost");
    }
}
