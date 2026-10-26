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

// One account a grant can name is the type of the account helpers
// (`exec_helper_protocol.rs`), so `se share exec accounts --json`, this reply
// and the bridge show the same choices (D.2.4).
pub(crate) use crate::share::exec_helper_protocol::AccountChoice;

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
    /// Rights and remaining routes for one exact pinned device.
    DeviceRights { device_id: String, public_key: String },
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
    DeviceRights { view: crate::share::access_relation_view::DeviceAccessView },
    ApprovalList {
        pending: Vec<ApprovalView>,
    },
    Decided,
    /// `crate::share::exec_account_choices()` of the worker.
    AccountList {
        helper: HelperStatus,
        choices: Vec<AccountChoice>,
    },
    ExecNotice {
        notice: Option<ExecNoticeView>,
    },
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
