//! Texts and failures a connecting device sees (C6, D.2.2): the error
//! codes of a refused start with their reason and next step, the status line
//! while a start waits for the host user, the account line of a session, and
//! the client's failure values. The host writes the refusal texts into
//! `ExecWireError.message`, so clients of every version show them; a client
//! of this version also replaces the bare text of hosts up to 0.5.182. The
//! CLI, MCP and the bridge are English; the German texts of the GUIs follow
//! C6 in their own files.
use std::{fmt, io};

use serde_json::{json, Value};

use crate::share::access_grant::Capability;
use crate::share::exec_access_view::{ApprovalSurface, ExecAccessView};
use crate::share::exec_client::{ExecClientFailure, ExecClientFailureKind};
use crate::share::types::PeerEndpoint;

/// A device without permission, or one whose grant ended or was withdrawn.
/// The reason stays open so that nobody learns whether a grant exists.
pub(crate) const PERMISSION_DENIED: &str = "permission_denied";
/// The grant allows the other capability, not the one the start needs.
pub(crate) const RIGHT_NOT_GRANTED: &str = "right_not_granted";
/// The host user refused the request.
pub(crate) const APPROVAL_DENIED: &str = "approval_denied";
/// The requesting device withdrew the request before a decision.
pub(crate) const APPROVAL_CANCELLED: &str = "approval_cancelled";
/// The host cannot show the request.
pub(crate) const APPROVAL_UNAVAILABLE: &str = "approval_unavailable";
/// The account of the grant is not allowed for remote access on the host.
pub(crate) const ACCOUNT_NOT_ALLOWED: &str = "account_not_allowed";
/// The account helper the grant's account needs is missing or outdated.
pub(crate) const ACCOUNT_HELPER_UNAVAILABLE: &str = "account_helper_unavailable";
/// `permission_denied` text of hosts up to 0.5.182.
pub(crate) const OLD_PERMISSION_DENIED_MESSAGE: &str = "exec authentication failed";

/// `permission_denied`: `device` is the host.
pub(crate) fn permission_denied(device: &str) -> String {
    format!(
        "{device} does not allow this device to run commands (not allowed, expired or \
         revoked). Next step: ask the owner of {device} to allow it."
    )
}

/// `right_not_granted` for a start that needs `missing`.
pub(crate) fn right_not_granted(device: &str, missing: Capability) -> String {
    let (granted, refused, option) = match missing {
        Capability::ExecTerminal => ("commands only", "no terminal", "Terminal"),
        _ => ("a terminal only", "no commands", "Commands"),
    };
    format!(
        "{device} allows this device {granted}, {refused}. Next step: ask the owner to \
         allow \"{option}\"."
    )
}

/// `approval_denied`.
pub(crate) fn approval_denied(device: &str) -> String {
    format!("{device}: access was refused on the device.")
}

/// `approval_cancelled`: the requesting device withdrew the request.
pub(crate) fn approval_cancelled(device: &str) -> String {
    format!("{device}: the request was withdrawn before anyone decided.")
}

/// `approval_unavailable` when notifications for Smart Explorer are off on
/// the host (C5).
pub(crate) fn approval_unavailable_notifications(device: &str) -> String {
    format!(
        "{device} cannot show the request: notifications for Smart Explorer are off. Next \
         step: allow notifications for Smart Explorer on {device}."
    )
}

/// `approval_unavailable` when the host has no way to ask its user.
pub(crate) fn approval_unavailable(device: &str) -> String {
    format!(
        "{device} cannot ask its user to confirm this start. Next step: ask the owner of \
         {device} to allow it without confirmation."
    )
}

/// `account_not_allowed`.
pub(crate) fn account_not_allowed(device: &str, account: &str) -> String {
    format!(
        "Account {account} is not allowed for remote access on {device}. Next step on the \
         device: Devices & sharing > Commands > Accounts > allow {account} (administrator \
         confirmation), or `se share exec accounts allow {account}`."
    )
}

/// `account_helper_unavailable`; `detail` says what is missing.
pub(crate) fn account_helper_unavailable(device: &str, account: &str, detail: &str) -> String {
    format!(
        "On {device} the account service is not set up or outdated ({detail}); commands as \
         {account} need it. Next step on the device: Devices & sharing > Commands > Accounts \
         > set up or update the account service."
    )
}

/// The name of a host in a client's texts: the name it announced, else its
/// device id.
pub(crate) fn peer_name(endpoint: &PeerEndpoint) -> String {
    let presence = &endpoint.presence;
    if presence.device_name.trim().is_empty() {
        presence.device_id.clone()
    } else {
        presence.device_name.clone()
    }
}

/// The text a client of this version shows for a refusal from `device`:
/// the host's own text, except the bare text of hosts up to 0.5.182.
pub(crate) fn client_failure_message(code: &str, message: &str, device: &str) -> String {
    if code == PERMISSION_DENIED && message == OLD_PERMISSION_DENIED_MESSAGE {
        permission_denied(device)
    } else {
        message.to_string()
    }
}

/// Status line while a start waits for the host user (`se shell`, `se exec`).
pub(crate) fn approval_status_line(device: &str, surface: Option<ApprovalSurface>) -> String {
    let only_cli = if surface == Some(ApprovalSurface::CliOnly) {
        " (on the device only with `se share exec approvals`)"
    } else {
        ""
    };
    format!("Waiting for confirmation on {device} ... (Ctrl+C cancels){only_cli}")
}

/// The JSON event of a waiting start.
pub(crate) fn approval_pending_event(device: &str, surface: Option<ApprovalSurface>) -> Value {
    json!({
        "event": "approval_pending",
        "device": device,
        "surface": surface.map(ApprovalSurface::key),
    })
}

/// The account line of a session (`se shell`, `se exec --verbose`, MCP job
/// status). `view` is `None` from hosts up to 0.5.182, which report only
/// the worker's account (`user_label`).
pub(crate) fn account_line(
    device: &str,
    view: Option<&ExecAccessView>,
    user_label: &str,
) -> String {
    match view {
        Some(view) => {
            let ask = if view.asks() {
                ", with confirmation"
            } else {
                ""
            };
            format!("{device}: as {}{ask}", view.account_label)
        }
        None => format!("{device}: account {user_label} (older host, rights not reported)"),
    }
}

pub(crate) fn authentication(
    error: impl fmt::Display,
    code: &str,
    start_sent: bool,
) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Authentication,
        code,
        &error.to_string(),
        start_sent,
    )
}

pub(crate) fn disconnected(error: impl fmt::Display, start_sent: bool) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Disconnected,
        "disconnected",
        &error.to_string(),
        start_sent,
    )
}

pub(crate) fn handshake_timeout(start_sent: bool) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Disconnected,
        "handshake_timeout",
        "Exec peer handshake timed out",
        start_sent,
    )
}

pub(crate) fn protocol(
    error: impl fmt::Display,
    start_sent: bool,
    code: &str,
) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Protocol,
        code,
        &error.to_string(),
        start_sent,
    )
}

pub(crate) fn classify_io(error: io::Error, start_sent: bool, code: &str) -> ExecClientFailure {
    match error.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => {
            protocol(error, start_sent, code)
        }
        io::ErrorKind::PermissionDenied => authentication(error, code, start_sent),
        _ => disconnected(error, start_sent),
    }
}

pub(crate) fn failure(
    kind: ExecClientFailureKind,
    code: &str,
    message: &str,
    start_may_have_been_sent: bool,
) -> ExecClientFailure {
    ExecClientFailure {
        kind,
        code: code.to_string(),
        message: message.to_string(),
        start_may_have_been_sent,
    }
}
