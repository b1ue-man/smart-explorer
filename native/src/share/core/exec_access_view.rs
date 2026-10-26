//! What a host of this version tells a connecting device before and during
//! a start (D.2.2, ALPN and protocol version unchanged): the access view in
//! `ExecHelloOk.access`, the features both sides announce, and the frame
//! `APPROVAL_PENDING` (39) of a start that waits for the host user. Older
//! peers ignore the new fields; the frame goes only to clients that announce
//! `exec_approval_v1`.
use serde::{Deserialize, Deserializer, Serialize};

use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_protocol::ExecHelloOk;
use crate::share::exec_types::ExecId;

/// Host feature: `ExecHelloOk.access` is sent.
pub(crate) const EXEC_ACCESS_FEATURE: &str = "exec_access_v1";
/// Client feature: it understands `APPROVAL_PENDING`; host feature: it can
/// ask its user before a start.
pub(crate) const EXEC_APPROVAL_FEATURE: &str = "exec_approval_v1";

/// `ExecClientHello.client_features` of this version (not in the proof).
pub(crate) fn client_features() -> Vec<String> {
    vec![EXEC_APPROVAL_FEATURE.into(), EXEC_ACCESS_FEATURE.into()]
}

/// The features a host of this version adds to `ExecHelloOk.features`.
pub(crate) fn host_access_features() -> [&'static str; 2] {
    [EXEC_ACCESS_FEATURE, EXEC_APPROVAL_FEATURE]
}

/// Whether the client announced that it understands `APPROVAL_PENDING`.
pub(crate) fn wants_approval_frames(client_features: &[String]) -> bool {
    client_features
        .iter()
        .any(|feature| feature == EXEC_APPROVAL_FEATURE)
}

/// The access right of the connecting device as the host tells it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExecAccessView {
    /// The account commands run as: the worker's account name, `se-remote`
    /// or the named account.
    pub(crate) account_label: String,
    /// `own`, `remote` or `account`.
    pub(crate) account_profile: String,
    pub(crate) commands: CapabilityDecision,
    pub(crate) terminal: CapabilityDecision,
    /// Only the programs of a list may start.
    pub(crate) programs_restricted: bool,
    #[serde(default)]
    pub(crate) valid_until: Option<i64>,
}

impl ExecAccessView {
    /// The view of `grant`; `own_label` names the worker's account.
    pub(crate) fn of(grant: &ExecGrant, own_label: &str) -> Self {
        let access = grant.access_grant();
        Self {
            account_label: access.account.label(own_label),
            account_profile: access.account.key().into(),
            commands: access.decision(Capability::ExecCommands),
            terminal: access.decision(Capability::ExecTerminal),
            programs_restricted: !grant.programs.is_empty(),
            valid_until: access.valid_until,
        }
    }

    /// A start on `ask` waits for the host user.
    pub(crate) fn asks(&self) -> bool {
        self.commands == CapabilityDecision::Ask || self.terminal == CapabilityDecision::Ask
    }
}

/// `HELLO_OK` of this host: `ExecHelloOk` plus `access` as its last key.
#[derive(Serialize)]
pub(crate) struct HelloOkWithAccess<'a> {
    #[serde(flatten)]
    pub(crate) hello: &'a ExecHelloOk,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) access: Option<&'a ExecAccessView>,
}

/// `HELLO_OK` as a client reads it. An access view it cannot read (a newer
/// host's decision values) is left out instead of failing the handshake.
#[derive(Deserialize)]
pub(crate) struct ReceivedHelloOk {
    #[serde(flatten)]
    pub(crate) hello: ExecHelloOk,
    #[serde(default, deserialize_with = "lenient_access")]
    pub(crate) access: Option<ExecAccessView>,
}

fn lenient_access<'de, D>(deserializer: D) -> Result<Option<ExecAccessView>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

/// What the host told a client beside the event stream: its access view,
/// and a start that waits for the host user (texts: `exec_client_messages`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExecClientNotice {
    /// The access view of a host of this version; `None` from older hosts.
    pub(crate) access: Option<ExecAccessView>,
    /// Set from `APPROVAL_PENDING` until the start is confirmed.
    pub(crate) approval: Option<ExecApprovalPending>,
}

/// Where the host asks its user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalSurface {
    /// The confirmation window was started.
    Window,
    /// Android, or a desktop without the window: a notification.
    Notification,
    /// No graphical session: only `se share exec approvals` answers.
    CliOnly,
}

impl ApprovalSurface {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Notification => "notification",
            Self::CliOnly => "cli_only",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        [Self::Window, Self::Notification, Self::CliOnly]
            .into_iter()
            .find(|surface| surface.key() == key)
    }
}

/// Payload of `APPROVAL_PENDING`: `{"exec_id","capability","surface"}`. The
/// values stay text so that a newer host's values never break the stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExecApprovalPending {
    pub(crate) exec_id: ExecId,
    pub(crate) capability: String,
    pub(crate) surface: String,
}

impl ExecApprovalPending {
    pub(crate) fn new(exec_id: ExecId, capability: Capability, surface: ApprovalSurface) -> Self {
        Self {
            exec_id,
            capability: capability.key().into(),
            surface: surface.key().into(),
        }
    }

    pub(crate) fn surface(&self) -> Option<ApprovalSurface> {
        ApprovalSurface::from_key(&self.surface)
    }
}

#[cfg(test)]
#[path = "exec_access_tests.rs"]
mod tests;

/// Contextual operations need an explicit V2 offer from the real protocol
/// handler. Legacy Exec/screen feature lists never imply these operations.
pub(crate) const ACCESS_CONSTRAINTS_V2_FEATURE: &str = "access_constraints_v2";
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AccessProtocolSupport {
    #[serde(default)]
    pub access_versions: Vec<u64>,
    #[serde(default)]
    pub contextual_capabilities: Vec<Capability>,
}
impl AccessProtocolSupport {
    pub(crate) fn host(available: &[Capability]) -> Self {
        Self {
            access_versions: vec![1, 2],
            contextual_capabilities: available.iter().copied()
                .filter(|capability| capability.requires_context()).collect(),
        }
    }
    pub(crate) fn supports(&self, request: &crate::share::access_constraints::AccessRequest) -> bool {
        self.access_versions.contains(&2) && self.contextual_capabilities.contains(&request.capability())
    }
}
