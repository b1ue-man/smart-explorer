//! Persistent Exec audit log of this host.
//!
//! `<App-Daten>/exec-audit.log` and `exec-audit-rejected.log` hold one JSON
//! object per line (files, classes and rotation: `exec_audit_files.rs`):
//! refused authentications, penalized EndpointIds, refused starts, starts and
//! their ends. Like OpenSSH's `auth.log`, it names who connected and what ran
//! (mode, program of an argv start, digest of the whole start, working
//! directory, account) but never the text of a shell command, which commonly
//! carries secrets; the digest matches the client's own record of the start.
//! Text the other side supplies is cut and cleaned (`exec_audit_text.rs`).
//! Writing is best effort: a full disk never blocks or fails an execution.
//!
//! Access rights (PRM-M4): the lines of refused starts, starts and ends add
//! `kind` (`exec`) and the capability; starts and ends add the account
//! profile and the confirmation that the admission noted (`note_start`).
//! Confirmations and ended grants are lines of their own
//! (`access_audit.rs`) in the file of authorized devices.
use std::io;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use serde_json::{json, Value};

use super::access_authorize::start_capability;
use super::access_grant::Capability;
use super::exec_types::{ExecCommand, ExecId, ExecJobView, ExecPrincipal, ExecStart};

#[path = "exec_audit_view.rs"]
mod audit_view;
#[path = "exec_audit_files.rs"]
mod files;
#[path = "exec_audit_text.rs"]
mod text;

pub(crate) use self::audit_view::{ExecAuditEntry, MAX_VIEW_ENTRIES};
use self::files::Class;
use self::text::limited;

/// Size at which a file of the log moves to `<name>.1` (4 MiB).
pub(crate) const ROTATE_AT: u64 = files::ROTATE_AT_BYTES;
/// Noted starts that no job entry claimed; beyond this the oldest goes.
const MAX_NOTED_STARTS: usize = 64;

/// What the access rights decided about one Exec start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartAccess {
    pub(crate) capability: Capability,
    /// `own`, `remote` or `account`.
    pub(crate) account_profile: &'static str,
    /// `not_required`, `once` or `always`.
    pub(crate) approval: &'static str,
}

static NOTED_STARTS: Mutex<Vec<(ExecId, StartAccess)>> = Mutex::new(Vec::new());

/// One audit event.
pub(crate) enum AuditEvent<'a> {
    /// The Exec handshake did not authenticate or authorize this peer.
    AuthRejected {
        remote_node: &'a str,
        claimed_device: &'a str,
        reason: &'a str,
    },
    /// A penalized EndpointId was turned away before any handshake.
    PenaltyRefused {
        remote_node: &'a str,
        protocol: &'a str,
        remaining_secs: u64,
    },
    /// An authorized peer's start was refused before any process existed.
    StartRefused {
        principal: &'a ExecPrincipal,
        start: &'a ExecStart,
        reason: &'a str,
    },
    /// A start was released to its contained process.
    Started {
        principal: &'a ExecPrincipal,
        start: &'a ExecStart,
        account: &'a str,
        /// The file a grant with a program list resolved the program to.
        resolved_program: Option<&'a str>,
    },
    /// The execution ended (the registry's final view).
    Finished {
        principal: &'a ExecPrincipal,
        view: &'a ExecJobView,
    },
}

impl AuditEvent<'_> {
    /// Strangers can produce `AuthRejected` and `PenaltyRefused` at will, so
    /// they never share a file with what authorized devices did.
    fn class(&self) -> Class {
        match self {
            Self::AuthRejected { .. } | Self::PenaltyRefused { .. } => Class::Rejections,
            Self::StartRefused { .. } | Self::Started { .. } | Self::Finished { .. } => {
                Class::Executions
            }
        }
    }
}

/// Notes what the access rights decided about the start `exec_id` before
/// its job runs; its `started` and `finished` entries carry it.
pub(crate) fn note_start(exec_id: &ExecId, access: StartAccess) {
    let mut noted = NOTED_STARTS.lock().unwrap_or_else(PoisonError::into_inner);
    noted.retain(|(id, _)| id != exec_id);
    noted.push((exec_id.clone(), access));
    let overflow = noted.len().saturating_sub(MAX_NOTED_STARTS);
    noted.drain(..overflow);
}

/// The start `exec_id` will not run (the registry refused it).
pub(crate) fn forget_start(exec_id: &ExecId) {
    NOTED_STARTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|(id, _)| id != exec_id);
}

fn noted_start(exec_id: &ExecId, take: bool) -> Option<StartAccess> {
    let mut noted = NOTED_STARTS.lock().unwrap_or_else(PoisonError::into_inner);
    let index = noted.iter().position(|(id, _)| id == exec_id)?;
    Some(if take {
        noted.remove(index).1
    } else {
        noted[index].1.clone()
    })
}

/// Appends `event` to the audit log of this installation.
pub(crate) fn record(event: AuditEvent<'_>) {
    let access = match &event {
        AuditEvent::Started { start, .. } => noted_start(&start.exec_id, false),
        AuditEvent::Finished { view, .. } => noted_start(&view.exec_id, true),
        _ => None,
    };
    let directory = crate::support_dirs::app_data_dir();
    let _ = files::append(
        &directory,
        event.class(),
        &line_with(&event, access.as_ref()),
    );
}

/// Appends one access-log line (`access_audit.rs`) to the file of
/// authorized devices; it rotates like every other line of that file.
pub(crate) fn append_access(directory: &Path, value: &Value, rotate_at: u64) -> io::Result<()> {
    files::append_rotating(directory, Class::Executions, value, rotate_at)
}

/// The `at` field of an entry written now.
pub(crate) fn now_text() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Text of the other side as an audit line keeps it.
pub(crate) fn clean(text: &str) -> String {
    limited(text)
}

/// Mode, program, digest, working directory and time limit of `start`.
pub(crate) fn exec_start_fields(start: &ExecStart) -> Value {
    start_fields(start)
}

/// `value` with the device fields every authorized entry has.
pub(crate) fn principal_fields(value: Value, principal: &ExecPrincipal) -> Value {
    with_principal(value, principal)
}

/// The last `lines` entries, oldest first, from every file of the log.
pub(crate) fn tail(lines: usize) -> io::Result<Vec<String>> {
    files::tail_in(&crate::support_dirs::app_data_dir(), lines)
}

/// The last `lines` entries (at most `MAX_VIEW_ENTRIES`) as the GUIs show
/// them: newest first, every field cut and cleaned.
pub(crate) fn view(lines: usize) -> io::Result<Vec<ExecAuditEntry>> {
    view_in(&crate::support_dirs::app_data_dir(), lines)
}

fn view_in(directory: &std::path::Path, lines: usize) -> io::Result<Vec<ExecAuditEntry>> {
    let lines = lines.clamp(1, MAX_VIEW_ENTRIES);
    Ok(audit_view::entries(files::tail_in(directory, lines)?))
}

#[cfg(test)]
fn line(event: &AuditEvent<'_>) -> Value {
    line_with(event, None)
}

fn line_with(event: &AuditEvent<'_>, access: Option<&StartAccess>) -> Value {
    let mut value = match event {
        AuditEvent::AuthRejected {
            remote_node,
            claimed_device,
            reason,
        } => json!({
            "event": "auth_rejected",
            "remote_node": limited(remote_node),
            "claimed_device": limited(claimed_device),
            "reason": limited(reason),
        }),
        AuditEvent::PenaltyRefused {
            remote_node,
            protocol,
            remaining_secs,
        } => json!({
            "event": "penalty_refused",
            "remote_node": limited(remote_node),
            "protocol": limited(protocol),
            "remaining_secs": remaining_secs,
        }),
        AuditEvent::StartRefused {
            principal,
            start,
            reason,
        } => {
            let mut value = start_fields(start);
            value["event"] = json!("start_refused");
            value["reason"] = json!(limited(reason));
            exec_fields(&mut value, Some(start_capability(&start.command)), None);
            with_principal(value, principal)
        }
        AuditEvent::Started {
            principal,
            start,
            account,
            resolved_program,
        } => {
            let mut value = start_fields(start);
            value["event"] = json!("started");
            value["account"] = json!(limited(account));
            if let Some(path) = resolved_program {
                value["resolved_program"] = json!(limited(path));
            }
            exec_fields(&mut value, Some(start_capability(&start.command)), access);
            with_principal(value, principal)
        }
        AuditEvent::Finished { principal, view } => {
            let terminal = view.terminal.as_ref();
            let mut value = json!({
                "event": "finished",
                "exec_id": view.exec_id.as_str(),
                "program": limited(&view.program),
                "digest": view.command_digest,
                "result": terminal.map(|terminal| format!("{:?}", terminal.kind).to_lowercase()),
                "exit_code": terminal.and_then(|terminal| terminal.exit_code),
                "signal": terminal.and_then(|terminal| terminal.signal),
                "stdout_bytes": terminal.map(|terminal| terminal.stdout_bytes),
                "stderr_bytes": terminal.map(|terminal| terminal.stderr_bytes),
                "duration_secs": view
                    .started_at
                    .zip(view.finished_at)
                    .map(|(started, finished)| finished.saturating_sub(started)),
            });
            exec_fields(&mut value, access.map(|access| access.capability), access);
            with_principal(value, principal)
        }
    };
    value["at"] = json!(now_text());
    value
}

/// PRM-M4: an entry about an Exec start names its kind and capability, and
/// how the access rights let it run.
fn exec_fields(value: &mut Value, capability: Option<Capability>, access: Option<&StartAccess>) {
    value["kind"] = json!("exec");
    if let Some(capability) = capability {
        value["capability"] = json!(capability.key());
    }
    if let Some(access) = access {
        value["account_profile"] = json!(access.account_profile);
        value["approval"] = json!(access.approval);
    }
}

fn start_fields(start: &ExecStart) -> Value {
    let mode = match &start.command {
        ExecCommand::Argv { .. } => "argv",
        ExecCommand::Shell { .. } => "shell",
        ExecCommand::Login => "terminal",
    };
    json!({
        "exec_id": start.exec_id.as_str(),
        "mode": mode,
        "program": limited(start.display_program()),
        "digest": start.digest().ok(),
        "cwd": start.cwd.as_deref().map(limited),
        "timeout_ms": start.effective_timeout_ms(),
    })
}

fn with_principal(mut value: Value, principal: &ExecPrincipal) -> Value {
    value["relation"] = json!(limited(&principal.relation_kind));
    value["relation_id"] = json!(limited(&principal.relation_id));
    value["device_id"] = json!(limited(&principal.device_id));
    value["device_name"] = json!(limited(&principal.device_name));
    value["fingerprint"] = json!(limited(&principal.fingerprint));
    value["node_id"] = json!(limited(&principal.node_id));
    value
}

/// What the PRM-M4 tests read of the log without the app data directory.
#[cfg(test)]
pub(crate) mod for_tests {
    use super::*;

    pub(crate) fn file_names() -> [(&'static str, &'static str); 2] {
        [Class::Executions.names(), Class::Rejections.names()]
    }

    pub(crate) fn line_of(event: &AuditEvent<'_>, access: Option<&StartAccess>) -> Value {
        line_with(event, access)
    }

    pub(crate) fn append_event(directory: &Path, event: &AuditEvent<'_>, rotate_at: u64) {
        files::append_rotating(directory, event.class(), &line(event), rotate_at).unwrap();
    }

    pub(crate) fn tail(directory: &Path, lines: usize) -> Vec<String> {
        files::tail_in(directory, lines).unwrap()
    }

    pub(crate) fn entries(directory: &Path, lines: usize) -> Vec<ExecAuditEntry> {
        view_in(directory, lines).unwrap()
    }

    pub(crate) fn noted(exec_id: &ExecId, take: bool) -> Option<StartAccess> {
        noted_start(exec_id, take)
    }
}

#[cfg(test)]
#[path = "exec_audit_tests.rs"]
mod tests;
