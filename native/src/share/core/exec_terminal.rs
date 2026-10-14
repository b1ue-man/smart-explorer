//! Optional terminal (PTY) request of one Exec start: its size and `TERM`
//! rules and the capability gates on both sides of the Exec protocol.
use std::io;

use serde::{Deserialize, Serialize};

use super::exec_client::{failure, ExecClientFailure, ExecClientFailureKind};
use super::exec_protocol::ExecWireError;
use super::exec_types::ExecStart;

/// Entry of `ExecHelloOk::features` that a host lists when it can give an Exec
/// start a terminal. A client never sends a terminal start without it.
pub(crate) const EXEC_TERMINAL_FEATURE: &str = "exec_terminal_v1";
/// Code of the host's `ServerFrame::Error` and of the client's failure for a
/// terminal start that the peer cannot serve.
pub(crate) const TERMINAL_UNSUPPORTED: &str = "terminal_unsupported";
/// ConPTY addresses its screen buffer with a signed 16-bit `COORD`.
const MAX_TERMINAL_DIMENSION: u16 = 32_767;
const MAX_TERM_LENGTH: usize = 64;

/// Window size of a terminal in character cells.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecTerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl ExecTerminalSize {
    pub fn validate(&self) -> io::Result<()> {
        let valid = 1..=MAX_TERMINAL_DIMENSION;
        if valid.contains(&self.cols) && valid.contains(&self.rows) {
            Ok(())
        } else {
            Err(invalid(
                "terminal columns and rows must be between 1 and 32767",
            ))
        }
    }
}

/// A terminal the host allocates for one start: the `TERM` value for the
/// remote environment and the initial window size.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecTerminalRequest {
    pub term: String,
    pub size: ExecTerminalSize,
}

impl ExecTerminalRequest {
    pub fn validate(&self) -> io::Result<()> {
        let term = self.term.as_bytes();
        if term.is_empty() || term.len() > MAX_TERM_LENGTH {
            return Err(invalid("terminal type must have 1 to 64 characters"));
        }
        let portable = |byte: &u8| byte.is_ascii_alphanumeric() || b"._+-".contains(byte);
        if !term.iter().all(portable) {
            return Err(invalid(
                "terminal type may only contain letters, digits and . _ + -",
            ));
        }
        self.size.validate()
    }
}

/// `ExecHelloOk::features` of a host that does (or does not) allocate terminals.
pub(crate) fn host_features(terminal_supported: bool) -> Vec<String> {
    if terminal_supported {
        vec![EXEC_TERMINAL_FEATURE.to_string()]
    } else {
        Vec::new()
    }
}

/// Host side: the error to answer when `start` wants a terminal that this
/// host cannot provide. No process may be prepared for such a start.
pub(crate) fn host_denial(start: &ExecStart, terminal_supported: bool) -> Option<ExecWireError> {
    (start.terminal.is_some() && !terminal_supported).then(|| ExecWireError {
        code: TERMINAL_UNSUPPORTED.into(),
        message: "this host cannot provide a terminal for remote commands".into(),
    })
}

/// Client side: a terminal start is only sent to a host that announced
/// `EXEC_TERMINAL_FEATURE`; an older host would reject the unknown field only
/// after the start had been sent.
pub(crate) fn require_host_terminal(
    start: &ExecStart,
    features: &[String],
) -> Result<(), ExecClientFailure> {
    if start.terminal.is_none() || features.iter().any(|name| name == EXEC_TERMINAL_FEATURE) {
        return Ok(());
    }
    Err(failure(
        ExecClientFailureKind::Remote,
        TERMINAL_UNSUPPORTED,
        "the peer's Smart Explorer has no terminal support; update it",
        false,
    ))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
