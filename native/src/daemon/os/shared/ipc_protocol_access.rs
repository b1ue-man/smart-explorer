//! The requests and replies inside `IpcRequest::Access` and
//! `IpcResponse::Access` (D.2.3, strand S1): access-right changes, open
//! confirmations and their answers, the account choices, and the notice of an
//! outgoing start (W10: it goes out here, `exec_ipc.rs` stays unchanged).
//! `ipc_protocol.rs` holds only the variant pair. A daemon of 0.5.182 refuses
//! the variant as unknown, so a newer client never mistakes it for an answer.
use serde::{Deserialize, Serialize};

use crate::daemon::ipc_host::exec_grant_journal::ExecGrantPersistResult;
use crate::share::access_approval::{ApprovalDecision, ApprovalView};
use crate::share::access_catalog::HelperStatus;
use crate::share::access_grant::{AccessGrantChange, AccessGrantView};
use crate::share::exec_access_view::{ExecAccessView, ExecApprovalPending};
use crate::share::{ExecGrantTarget, ExecId};

/// One request of the access rights.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AccessIpcRequest {
    /// Changes account, capabilities, program list or end of one grant.
    /// `expected_revision` is the revision the surface showed.
    MutateGrant {
        target: ExecGrantTarget,
        change: AccessGrantChange,
        #[serde(default)]
        expected_revision: Option<u64>,
    },
    /// The open confirmations of this host.
    Approvals,
    /// The answer of the host user to exactly one request.
    Decide {
        approval_id: String,
        decision: ApprovalDecision,
    },
    /// The Android app could not show the request (notifications off).
    ReportUndeliverable { approval_id: String },
    /// The accounts a grant can name on this host.
    Accounts,
    /// What the host of the outgoing start `exec_id` told beside its events.
    ExecNotice { exec_id: ExecId },
}

/// The replies of `AccessIpcRequest`, in the same order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AccessIpcReply {
    /// `grant` is the grant as requested; `None` while an earlier change is
    /// still being recovered (then `result` is that one).
    GrantMutation {
        result: ExecGrantPersistResult,
        grant: Option<AccessGrantView>,
    },
    ApprovalList {
        pending: Vec<ApprovalView>,
    },
    Decided,
    AccountList {
        helper: HelperStatus,
        choices: Vec<AccountChoice>,
    },
    ExecNotice {
        notice: Option<ExecNoticeView>,
    },
}

/// One account a grant can name (D.2.4 `se share exec accounts --json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AccountChoice {
    /// `own`, `remote` or `account`.
    pub(crate) profile: String,
    /// How surfaces name the own account (the worker's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    /// The local account of `remote` and `account`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) account: Option<String>,
    pub(crate) available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) next_step: Option<String>,
    #[serde(default)]
    pub(crate) elevated: bool,
    #[serde(default)]
    pub(crate) admin_group: bool,
}

impl AccountChoice {
    /// The worker's own account.
    pub(crate) fn own(label: &str) -> Self {
        Self {
            profile: "own".into(),
            label: Some(label.to_string()),
            account: None,
            available: true,
            reason: None,
            next_step: None,
            elevated: false,
            admin_group: false,
        }
    }

    /// An account a grant of this host already names.
    pub(crate) fn account(name: &str) -> Self {
        Self {
            profile: "account".into(),
            label: None,
            account: Some(name.to_string()),
            ..Self::own(name)
        }
    }
}

/// The notice of one outgoing start (D.2.2): the host's access view and a
/// confirmation the start waits for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExecNoticeView {
    /// The peer as this device names it; empty when the job is not listed.
    pub(crate) device: String,
    pub(crate) access: Option<ExecAccessView>,
    pub(crate) approval: Option<ExecApprovalPending>,
}
