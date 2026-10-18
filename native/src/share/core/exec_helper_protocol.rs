//! Account helper protocol (plan 2026-10-17 „Rechte, se Desktop, Tresore",
//! contract D.2.7). A worker asks the account helper of its host to run a
//! job's supervisor as another local account: on Linux the root socket
//! service `smart-explorer-exec-helper` (PRM-M6), on Windows the service
//! `SmartExplorerExecAccounts` (PRM-M7). A frame is the 4-byte big-endian
//! length of a JSON object of at most 64 KiB, then the object. The first
//! request of a connection is `hello`; every session a connection started
//! ends when the connection closes.
//!
//! The file also holds what the surfaces show for "Ausführen als" (D.2.4:
//! `se share exec accounts --json`, IPC `Accounts`, bridge
//! `accountChoices`) and the error a refused account start hands to the
//! Exec admission.
// Android has no account helper (C4) and reads only the account choices and
// the refusal codes; the lint stays active on Linux and Windows, whose
// helpers (PRM-M6, PRM-M7) use every item.
#![cfg_attr(target_os = "android", allow(dead_code))]

use std::fmt;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::share::access_catalog::HelperStatus;
use crate::share::exec_client_messages::{ACCOUNT_HELPER_UNAVAILABLE, ACCOUNT_NOT_ALLOWED};

/// Version of this protocol. It also covers the supervisor protocol
/// (`exec_supervisor_protocol.rs`) that runs over a session's descriptor or
/// pipes: a change of either that an older peer cannot read raises it, and
/// a worker then reports its helper as outdated.
pub const HELPER_PROTOCOL: u32 = 1;
/// Largest JSON object of one frame.
pub const MAX_HELPER_FRAME_BYTES: usize = 64 * 1024;
/// `ExecWireError.code` of a start whose account the host cannot use (the
/// code hosts up to 0.5.182 send for a missing run-as account).
pub const ACCOUNT_UNAVAILABLE: &str = "account_unavailable";

/// A request of a worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperRequest {
    /// First request of every connection.
    Hello { protocol: u32, se_version: String },
    /// Whether `account` could run a session, without starting one.
    Probe { account: String },
    /// Runs a supervisor as `account`. On Linux the frame carries the
    /// supervisor's end of a socket pair (`SCM_RIGHTS`); on Windows it names
    /// the worker's two pipes.
    Start {
        account: String,
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pipe_in: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pipe_out: Option<String>,
    },
    /// Ends every process of a session of this connection.
    Stop { session: String },
    /// Whether a session of this connection still runs.
    Status { session: String },
}

impl HelperRequest {
    /// `hello` of this program.
    pub fn hello() -> Self {
        Self::Hello {
            protocol: HELPER_PROTOCOL,
            se_version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

/// The helper's answer to one request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperReply {
    HelloOk {
        protocol: u32,
        se_version: String,
    },
    Probe {
        ok: bool,
        /// The account would run with administrator rights.
        elevated: bool,
        /// The account belongs to an administrator group.
        admin_group: bool,
        /// Why `ok` is false, or a warning about a usable account.
        detail: Option<String>,
    },
    Started {
        session: String,
    },
    Stopped {},
    Status {
        active: bool,
        empty: bool,
    },
    Error {
        code: HelperErrorCode,
        detail: String,
    },
}

/// Error codes of the helper (D.2.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperErrorCode {
    /// The caller or the account is not in the helper's configuration.
    NotAllowed,
    UnknownAccount,
    LogonFailed,
    PasswordChangeRequired,
    /// Protocol, program or configuration of the helper do not fit the
    /// worker; the helper needs an update.
    Outdated,
    StartFailed,
    BadRequest,
    /// A code of a newer helper.
    #[serde(other)]
    Other,
}

impl HelperErrorCode {
    /// The `ExecWireError.code` of a start this error refused (D.2.2).
    pub fn wire_code(self) -> &'static str {
        match self {
            Self::NotAllowed => ACCOUNT_NOT_ALLOWED,
            Self::Outdated => ACCOUNT_HELPER_UNAVAILABLE,
            _ => ACCOUNT_UNAVAILABLE,
        }
    }
}

/// One frame: the 4-byte big-endian length of the JSON object, then the
/// object.
pub fn encode_frame<T: Serialize>(message: &T) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(message).map_err(io::Error::other)?;
    if !(1..=MAX_HELPER_FRAME_BYTES).contains(&payload.len()) {
        return Err(invalid("an account helper frame exceeds 64 KiB".into()));
    }
    let length = u32::try_from(payload.len())
        .map_err(|_| invalid("an account helper frame exceeds 64 KiB".into()))?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Length of the object a frame header announces; empty and oversized
/// frames are refused before anything is allocated.
pub fn frame_length(header: [u8; 4]) -> io::Result<usize> {
    usize::try_from(u32::from_be_bytes(header))
        .ok()
        .filter(|length| (1..=MAX_HELPER_FRAME_BYTES).contains(length))
        .ok_or_else(|| invalid("an account helper frame has an invalid length".into()))
}

pub fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload)
        .map_err(|error| invalid(format!("an account helper frame is not valid: {error}")))
}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    writer.write_all(&encode_frame(message)?)?;
    writer.flush()
}

/// Reads one frame; `Ok(None)` when the peer closed before a new frame.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<Option<T>> {
    let mut header = [0u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        match reader.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "an account helper frame ends early",
                ))
            }
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let mut payload = vec![0u8; frame_length(header)?];
    reader.read_exact(&mut payload)?;
    decode_payload(&payload).map(Some)
}

/// Profile of an account choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountChoiceProfile {
    Own,
    Remote,
    Account,
}

/// Why a choice is not available, or what the warning of an available one
/// is about. The GUIs show their own texts for it (C6); CLI, JSON and the
/// bridge show `reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountReason {
    /// There is no such local account.
    NotFound,
    /// Remote commands never run as root (Linux), nor as an administrator
    /// or system account (Windows), through another account.
    Root,
    /// The account service does not list the account for this user.
    NotAllowed,
    HelperNotInstalled,
    HelperOutdated,
    /// No systemd with cgroup v2, or a platform without other accounts.
    Unsupported,
    PasswordChangeRequired,
    Expired,
    /// Warning only: the account's login shell runs no commands.
    NoLoginShell,
    /// Windows refused the logon without password (S4U) of the account's
    /// probe; `reason` names the Windows error (C2, C6).
    LogonRefused,
}

/// One entry of "Ausführen als" (D.2.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountChoice {
    pub profile: AccountChoiceProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The account's name; for `own` the worker's account.
    pub label: String,
    pub available: bool,
    /// Why the account is not available; for an available account a warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<AccountReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    /// The account would run with administrator rights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevated: Option<bool>,
    /// The account belongs to an administrator group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_group: Option<bool>,
}

impl AccountChoice {
    /// "Mein Konto": the worker's own account, always available.
    pub fn own(label: &str) -> Self {
        Self {
            profile: AccountChoiceProfile::Own,
            account: None,
            label: label.into(),
            available: true,
            reason: None,
            reason_code: None,
            next_step: None,
            elevated: None,
            admin_group: None,
        }
    }

    /// Another account (`remote` or `account`); available until refused.
    pub fn other(profile: AccountChoiceProfile, account: &str) -> Self {
        Self {
            profile,
            account: Some(account.into()),
            label: account.into(),
            available: true,
            reason: None,
            reason_code: None,
            next_step: None,
            elevated: Some(false),
            admin_group: Some(false),
        }
    }

    pub fn refuse(&mut self, code: AccountReason, reason: String, next_step: Option<String>) {
        self.available = false;
        self.reason_code = Some(code);
        self.reason = Some(reason);
        self.next_step = next_step;
    }

    /// A warning that leaves the account available.
    pub fn warn(&mut self, code: AccountReason, reason: String) {
        self.reason_code = Some(code);
        self.reason = Some(reason);
    }
}

/// The account helper of this host and every account choice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountChoices {
    pub helper: HelperStatus,
    pub choices: Vec<AccountChoice>,
}

/// A start as another account that this host refused; `code` is the
/// `ExecWireError.code` the admission sends (D.2.2).
#[derive(Debug)]
pub struct AccountStartError {
    pub code: &'static str,
    pub detail: String,
}

impl fmt::Display for AccountStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for AccountStartError {}

/// An `io::Error` that carries `code` to the admission.
pub fn account_start_error(code: &'static str, detail: impl Into<String>) -> io::Error {
    let kind = if code == ACCOUNT_NOT_ALLOWED {
        io::ErrorKind::PermissionDenied
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(
        kind,
        AccountStartError {
            code,
            detail: detail.into(),
        },
    )
}

/// The `ExecWireError.code` of `error` when it refused an account start.
pub fn account_error_code(error: &io::Error) -> Option<&'static str> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<AccountStartError>())
        .map(|refused| refused.code)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "exec_helper_protocol_tests.rs"]
mod tests;
