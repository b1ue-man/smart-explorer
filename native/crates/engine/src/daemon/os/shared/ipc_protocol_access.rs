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
use crate::share::connection_rights::{ConnectionRights, PresetView, RightsTarget};
use crate::share::connection_rights_apply::PendingRightsView;
use crate::share::exec_access_view::{ExecAccessView, ExecApprovalPending};
use crate::share::{ExecGrantTarget, ExecId};

// One account a grant can name is the type of the account helpers
// (`exec_helper_protocol.rs`), so `se share exec accounts --json`, this reply
// and the bridge show the same choices (D.2.4).
pub use crate::share::exec_helper_protocol::AccountChoice;

/// One request of the access rights.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccessIpcRequest {
    /// Changes account, capabilities, program list or end of one grant.
    /// `expected_revision` is the revision the surface showed.
    MutateGrant {
        target: ExecGrantTarget,
        change: AccessGrantChange,
        #[serde(default)]
        expected_revision: Option<u64>,
    },
    /// Rights and remaining routes for one exact pinned device.
    DeviceRights {
        device_id: String,
        public_key: String,
    },
    /// The open confirmations of this host.
    Approvals,
    /// The answer of the host user to one request; „Immer erlauben“ and
    /// „Ablehnen“ also answer the open requests equal to it (E28).
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
    /// F4 (plan 2026-10-31): the rights presets in menu order.
    RightsPresets,
    /// F1: the connections whose rights wait („Rechte offen“) and the rights
    /// of the Direct code.
    PendingRights,
    /// F1 „Bestätigen“: exactly `rights` for `target`, all or nothing; the
    /// folders in `added` (label, path) become Direct exports in the same
    /// step. A change of commands, terminal or screen goes through the
    /// Exec-grant journal (W8, review 2026-10-31).
    ConfirmRights {
        target: RightsTarget,
        rights: ConnectionRights,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        added: Vec<(String, String)>,
    },
    /// F1 „Abbrechen“: the connection stays without rights.
    CancelRights { target: RightsTarget },
    /// F5/F6: the rights of devices connecting with the Direct code;
    /// `None` lets them wait without rights.
    SetCodeRights {
        #[serde(default)]
        rights: Option<ConnectionRights>,
    },
}

/// The replies of `AccessIpcRequest`, in the same order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccessIpcReply {
    /// `grant` is the grant as requested; `None` while an earlier change is
    /// still being recovered (then `result` is that one).
    GrantMutation {
        result: ExecGrantPersistResult,
        grant: Option<AccessGrantView>,
    },
    DeviceRights {
        view: crate::share::access_relation_view::DeviceAccessView,
    },
    ApprovalList {
        pending: Vec<ApprovalView>,
    },
    /// The answer was taken. `also_answered` counts the open requests equal to
    /// the answered one that were answered too (E28); „Diesmal erlauben“ always
    /// counts 0, and a worker that sends no count reads as 0.
    Decided {
        #[serde(default)]
        also_answered: usize,
    },
    /// `crate::share::exec_account_choices()` of the worker.
    AccountList {
        helper: HelperStatus,
        choices: Vec<AccountChoice>,
    },
    ExecNotice {
        notice: Option<ExecNoticeView>,
    },
    RightsPresets {
        presets: Vec<PresetView>,
    },
    /// Reply of `PendingRights`, and of every rights change (`changed`
    /// tells whether it changed anything; `false` for `PendingRights`).
    Rights {
        pending: Vec<PendingRightsView>,
        code_rights: Option<ConnectionRights>,
        #[serde(default)]
        changed: bool,
    },
}

/// The notice of one outgoing start (D.2.2): the host's access view and a
/// confirmation the start waits for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecNoticeView {
    /// The peer as this device names it; empty when the job is not listed.
    pub device: String,
    pub access: Option<ExecAccessView>,
    pub approval: Option<ExecApprovalPending>,
}

#[cfg(test)]
#[path = "access_ipc_decided_task_tests.rs"]
mod decided_task_tests;
