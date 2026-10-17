//! Access-log entries of the access rights (PRM-M4, C5, contract V7 point
//! 5): confirmations, grants that ended and refusals of the account helper,
//! in the files of the Exec audit log (`exec-audit.log` with its rotation,
//! `exec_audit_files.rs`). Every line names the device like the Exec lines,
//! its `kind` (`exec`, or `screen` from S4), the capability and, where they
//! apply, the account and the confirmation (`not_required`, `once`, `always`,
//! `denied`, `cancelled`, `unavailable`). An Exec line carries mode, program
//! and digest of the start, never the text of a shell command; text of the
//! other device is cut and cleaned like in every audit line. Writing is best
//! effort: a full disk never blocks or fails a start.
use serde_json::{json, Value};

use crate::share::access_grant::Capability;
use crate::share::exec_audit;
use crate::share::exec_types::ExecPrincipal;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::ExecStart;

/// What an entry is about.
pub(crate) enum AccessKind<'a> {
    /// An Exec start (`None`: an entry about the grant, not one start).
    Exec { start: Option<&'a ExecStart> },
    /// A screen session of S4: `mode` (`view`, `control`) and, for an
    /// entry without `outcome`, the name of the session event.
    #[allow(dead_code)] // Staged: U10 (SCR-M2) logs screen sessions.
    Screen { mode: &'a str, event: &'a str },
}

impl AccessKind<'_> {
    fn key(&self) -> &'static str {
        match self {
            Self::Exec { .. } => "exec",
            Self::Screen { .. } => "screen",
        }
    }
}

/// The events of the access rights (C5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessEvent {
    ApprovalRequested,
    ApprovalGranted,
    ApprovalDenied,
    ApprovalCancelled,
    ApprovalUnavailable,
    GrantExpired,
    #[allow(dead_code)] // Staged: the account helper (U18, U24) logs its refusals.
    HelperRefused,
}

impl AccessEvent {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::ApprovalRequested => "approval_requested",
            Self::ApprovalGranted => "approval_granted",
            Self::ApprovalDenied => "approval_denied",
            Self::ApprovalCancelled => "approval_cancelled",
            Self::ApprovalUnavailable => "approval_unavailable",
            Self::GrantExpired => "grant_expired",
            Self::HelperRefused => "helper_refused",
        }
    }
}

/// How a start or session was confirmed (`approval` of an entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalResult {
    /// The capability was `allow`.
    NotRequired,
    Once,
    Always,
    Denied,
    Cancelled,
    Unavailable,
}

impl ApprovalResult {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Once => "once",
            Self::Always => "always",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One access-log entry (V7: `record_access(AccessEntry { kind, principal,
/// outcome, approval, … })`).
pub(crate) struct AccessEntry<'a> {
    pub(crate) kind: AccessKind<'a>,
    pub(crate) principal: &'a ProvenPrincipal,
    /// What happened; `None` for a screen-session event named by `kind`.
    pub(crate) outcome: Option<AccessEvent>,
    pub(crate) capability: Capability,
    pub(crate) approval: Option<ApprovalResult>,
    pub(crate) approval_id: Option<&'a str>,
    /// `own`, `remote` or `account` (Exec capabilities only).
    pub(crate) account_profile: Option<&'a str>,
    /// The account an Exec start runs as.
    pub(crate) account: Option<&'a str>,
    pub(crate) reason: Option<&'a str>,
}

/// Appends `entry` to the audit log of this installation.
pub(crate) fn record_access(entry: AccessEntry<'_>) {
    let directory = crate::support_dirs::app_data_dir();
    let _ = exec_audit::append_access(&directory, &access_line(&entry), exec_audit::ROTATE_AT);
}

/// The audit line of `entry`.
pub(crate) fn access_line(entry: &AccessEntry<'_>) -> Value {
    let mut value = match &entry.kind {
        AccessKind::Exec { start: Some(start) } => exec_audit::exec_start_fields(start),
        AccessKind::Exec { start: None } => json!({}),
        AccessKind::Screen { mode, .. } => json!({ "mode": exec_audit::clean(mode) }),
    };
    let event = match (entry.outcome, &entry.kind) {
        (Some(outcome), _) => outcome.key().to_string(),
        (None, AccessKind::Screen { event, .. }) => exec_audit::clean(event),
        (None, AccessKind::Exec { .. }) => "access".to_string(),
    };
    value["event"] = json!(event);
    value["kind"] = json!(entry.kind.key());
    value["capability"] = json!(entry.capability.key());
    if let Some(approval) = entry.approval {
        value["approval"] = json!(approval.key());
    }
    let texts = [
        ("approval_id", entry.approval_id),
        ("account_profile", entry.account_profile),
        ("account", entry.account),
        ("reason", entry.reason),
    ];
    for (key, text) in texts
        .into_iter()
        .filter_map(|(key, text)| text.map(|text| (key, text)))
    {
        value[key] = json!(exec_audit::clean(text));
    }
    let mut value = exec_audit::principal_fields(value, &ExecPrincipal::from(entry.principal));
    value["at"] = json!(exec_audit::now_text());
    value
}

#[cfg(test)]
#[path = "access_audit_tests.rs"]
mod tests;
