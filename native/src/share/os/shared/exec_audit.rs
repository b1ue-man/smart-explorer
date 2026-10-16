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
use std::io;

use serde_json::{json, Value};

use super::exec_types::{ExecCommand, ExecJobView, ExecPrincipal, ExecStart};

#[path = "exec_audit_files.rs"]
mod files;
#[path = "exec_audit_text.rs"]
mod text;

use self::files::Class;
use self::text::limited;

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

/// Appends `event` to the audit log of this installation.
pub(crate) fn record(event: AuditEvent<'_>) {
    let directory = crate::support_dirs::app_data_dir();
    let _ = files::append(&directory, event.class(), &line(&event));
}

/// The last `lines` entries, oldest first, from every file of the log.
pub(crate) fn tail(lines: usize) -> io::Result<Vec<String>> {
    files::tail_in(&crate::support_dirs::app_data_dir(), lines)
}

fn line(event: &AuditEvent<'_>) -> Value {
    let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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
            with_principal(value, principal)
        }
        AuditEvent::Finished { principal, view } => {
            let terminal = view.terminal.as_ref();
            let value = json!({
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
            with_principal(value, principal)
        }
    };
    value["at"] = json!(at);
    value
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

#[cfg(test)]
#[path = "exec_audit_tests.rs"]
mod tests;
