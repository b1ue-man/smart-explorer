//! Waiting for the answer of a long vault request (user rule 2026-11-17: no
//! fixed total time on an operation that still runs).
//!
//! The worker answers a comparison, a check or an action only when it is
//! done; a large vault over a network takes long without any sign in
//! between, and the work ends as soon as the client closes its connection
//! (`ipc_vault_compare.rs`, `watch_client`). The former 600 s client wait
//! therefore ended the work itself. The client now waits for the first byte
//! of the answer as long as its connection is open and the worker that
//! took the request still answers `Ping` with the same generation. It stops
//! when the connection closes, when another worker answers (the comparison
//! went with the old one) or when no worker answered `Ping` three looks in
//! a row (about one and a half minutes). Once the answer flows, each read
//! has `READ_IDLE`.
use std::io;
use std::net::TcpStream;
use std::time::Duration;

use super::ipc_protocol::{read_response, write_request, IpcRequest, IpcResponse};
use super::ipc_storage::{read_ipc_addr, read_token};

/// How long the client waits for the answer between two looks at the worker.
const LOOK_EVERY: Duration = Duration::from_secs(30);
/// Time of one `Ping` to the worker.
const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// Time one read or write may take once the request or the answer flows.
pub(super) const READ_IDLE: Duration = Duration::from_secs(60);

/// What one look at the worker means for the wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Look {
    Wait,
    /// Another worker answers: the request went with the one before.
    Replaced,
    /// No worker answered `MAX_SILENT` looks in a row.
    Silent,
}

/// The looks at the worker during one wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkerWatch {
    first: Option<String>,
    silent: u32,
}

impl WorkerWatch {
    pub(super) const MAX_SILENT: u32 = 3;

    /// `first`: the generation of the worker that took the request, when
    /// known; otherwise the first answer counts.
    pub(super) fn new(first: Option<String>) -> Self {
        Self { first, silent: 0 }
    }

    /// `now`: the generation that answered `Ping`; `None`: no answer.
    pub(super) fn look(&mut self, now: Option<&str>) -> Look {
        let Some(now) = now else {
            self.silent = self.silent.saturating_add(1);
            return if self.silent >= Self::MAX_SILENT {
                Look::Silent
            } else {
                Look::Wait
            };
        };
        self.silent = 0;
        match self.first.as_deref() {
            Some(first) if first != now => Look::Replaced,
            Some(_) => Look::Wait,
            None => {
                self.first = Some(now.to_string());
                Look::Wait
            }
        }
    }
}

/// The generation of the worker that answers `Ping` now; `None`: none did.
fn worker_generation() -> Option<String> {
    let addr = read_ipc_addr()?;
    let token = read_token().ok()?;
    let mut stream = crate::daemon::ipc_proof::connect(&addr, PING_TIMEOUT).ok()?;
    let _ = stream.set_read_timeout(Some(PING_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PING_TIMEOUT));
    write_request(&mut stream, &IpcRequest::Ping { token }).ok()?;
    match read_response(&mut stream).ok()? {
        IpcResponse::Pong { generation, .. } if !generation.is_empty() => Some(generation),
        _ => None,
    }
}

/// Why a wait ended without the answer.
#[derive(Debug)]
pub(super) enum Stopped {
    /// The worker closed the connection.
    Closed,
    Replaced,
    Silent,
    Failed(io::Error),
}

/// Waits until the first byte of the answer on `stream` is there, without
/// a time limit while the worker lives (see the module text).
pub(super) fn first_byte(stream: &TcpStream) -> Result<(), Stopped> {
    let mut watch = WorkerWatch::new(None);
    let _ = stream.set_read_timeout(Some(LOOK_EVERY));
    let mut byte = [0u8; 1];
    loop {
        match stream.peek(&mut byte) {
            Ok(0) => return Err(Stopped::Closed),
            Ok(_) => {
                let _ = stream.set_read_timeout(Some(READ_IDLE));
                return Ok(());
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                match watch.look(worker_generation().as_deref()) {
                    Look::Wait => {}
                    Look::Replaced => return Err(Stopped::Replaced),
                    Look::Silent => return Err(Stopped::Silent),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(Stopped::Failed(error)),
        }
    }
}

#[cfg(test)]
#[path = "ipc_vault_compare_wait_tests.rs"]
mod tests;
