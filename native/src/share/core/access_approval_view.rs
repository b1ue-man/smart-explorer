//! What a confirmation request shows and how the host user answers it
//! (PRM-M3, D.2.3, D.2.4): its id, the decision `once | always | deny`, the
//! command of an Exec start and the view that the consent window, `se share
//! exec approvals` and the Android app list. Text of the requesting device
//! is cut to 256 characters and loses every control and direction character,
//! so it can neither rearrange the dialog nor reach the audit log as such.
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::share::access_constraints::AccessRequest;
use crate::share::access_grant::Capability;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::ExecCommand;

/// Longest text of the requesting device a request shows (C5).
const MAX_SHOWN_CHARS: usize = 256;
/// Arguments of an argv start a request lists; more are counted.
const MAX_SHOWN_ARGS: usize = 32;

/// Id of one request: 16 random bytes as 32 lowercase hex characters. A
/// decision names exactly one request; ids are never reused.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct ApprovalId(String);

impl ApprovalId {
    pub(crate) fn generate() -> Result<Self, String> {
        crate::share::core::random_hex_token::<16>().map(Self)
    }

    /// An id from outside (IPC, command line, window argument).
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        if text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(text.to_ascii_lowercase()))
        } else {
            Err("approval id must be exactly 32 hexadecimal characters".into())
        }
    }

    /// The id of a request that could not be created (no random bytes).
    pub(crate) fn unset() -> Self {
        Self("0".repeat(32))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ApprovalId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}

impl From<ApprovalId> for String {
    fn from(id: ApprovalId) -> Self {
        id.0
    }
}

impl fmt::Display for ApprovalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The host user's answer: „Diesmal erlauben“, „Immer erlauben“, „Ablehnen“.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ApprovalDecision {
    Once,
    Always,
    Deny,
}

/// The command of an Exec start as a request shows it (never logged).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum ApprovalCommand {
    Argv { program: String, args: Vec<String> },
    Shell { text: String },
    Login,
}

impl ApprovalCommand {
    pub(crate) fn of(command: &ExecCommand) -> Self {
        match command {
            ExecCommand::Argv { program, args } => {
                let mut listed: Vec<String> = args
                    .iter()
                    .take(MAX_SHOWN_ARGS)
                    .map(|arg| shown(arg))
                    .collect();
                if args.len() > MAX_SHOWN_ARGS {
                    listed.push(format!("… (+{})", args.len() - MAX_SHOWN_ARGS));
                }
                Self::Argv {
                    program: shown(program),
                    args: listed,
                }
            }
            ExecCommand::Shell { command } => Self::Shell {
                text: shown(command),
            },
            ExecCommand::Login => Self::Login,
        }
    }
}

/// What the requester tells about its request besides principal and
/// capability (contract V7: `request(principal, capability, summary)`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ApprovalSummary {
    /// The name this host stored for the device; the name the device gave
    /// itself when empty.
    pub(crate) device_name: Option<String>,
    /// Name of the room of a room member.
    pub(crate) room: Option<String>,
    /// The account an Exec start runs as; `None` for the screen.
    pub(crate) account_label: Option<String>,
    /// The command of an Exec start; `None` for the screen.
    pub(crate) command: Option<ApprovalCommand>,
    /// Revision of the grant the request is made under. „Immer erlauben“
    /// changes exactly this revision (compare and swap).
    pub(crate) policy_revision: u64,
    /// A client without `exec_approval_v1` asks for a terminal: it waits at
    /// most 60 seconds (C5).
    pub(crate) older_client_terminal: bool,
}

/// One open request (D.2.4 `se share exec approvals --json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ApprovalView {
    pub(crate) id: ApprovalId,
    pub(crate) device_name: String,
    pub(crate) device_id: String,
    pub(crate) fingerprint: String,
    /// `direct` or `room`.
    pub(crate) relation: String,
    /// Exact proven policy source; display names never identify a relation.
    #[serde(default)]
    pub(crate) relation_id: String,
    pub(crate) room: Option<String>,
    pub(crate) capability: Capability,
    pub(crate) account_label: Option<String>,
    pub(crate) command: Option<ApprovalCommand>,
    /// Unix seconds (host clock).
    pub(crate) requested_at: i64,
    /// `window`, `notification` or `cli_only`; `None` while it is shown.
    pub(crate) surface: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) older_client_terminal: bool,
    /// Exact V2 target shown to the human; absent for legacy Exec/screen requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request: Option<AccessRequest>,
}

impl ApprovalView {
    pub(crate) fn new(
        id: &ApprovalId,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: &ApprovalSummary,
        now: i64,
    ) -> Self {
        let named = summary
            .device_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&principal.device_name);
        let device_name = if named.trim().is_empty() {
            &principal.device_id
        } else {
            named
        };
        Self {
            id: id.clone(),
            device_name: shown(device_name),
            device_id: shown(&principal.device_id),
            fingerprint: shown(&principal.fingerprint),
            relation: shown(&principal.relation_kind),
            relation_id: principal.relation_id.clone(),
            room: summary.room.as_deref().map(shown),
            capability,
            account_label: summary.account_label.as_deref().map(shown),
            command: summary.command.clone(),
            requested_at: now,
            surface: None,
            older_client_terminal: summary.older_client_terminal,
            request: None,
        }
    }
}

/// Text of the requesting device as a request shows it.
fn shown(text: &str) -> String {
    let mut shown = String::with_capacity(text.len().min(MAX_SHOWN_CHARS * 4));
    for (index, character) in text.chars().enumerate() {
        if index == MAX_SHOWN_CHARS {
            shown.push('…');
            break;
        }
        shown.push(if misleading(character) {
            '?'
        } else {
            character
        });
    }
    shown
}

fn misleading(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}
