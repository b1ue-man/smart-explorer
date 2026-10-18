//! Daemon side of `IpcRequest::Access` (D.2.3, strand S1) and the client
//! calls for it. The variant reaches `ShareHost::serve_access` through
//! `ipc_host_ext.rs` after its daemon token was checked (W10:
//! `exec_ipc.rs` stays unchanged). „Immer erlauben“ changes the grant through
//! the grant journal at the revision the request was made under, before the
//! waiting start is released; any failure leaves the request open. The
//! notice of an outgoing start (D.2.2) goes out here too: the CLI and MCP read
//! it while their start waits for the other device's user.
use std::collections::BTreeMap;
use std::io;
use std::net::TcpStream;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use crate::daemon::ipc_host::exec_grant_journal::ExecGrantPersistResult;
use crate::daemon::ipc_host::ShareHost;
use crate::daemon::ipc_protocol::{
    read_response, set_stream_timeout, write_request, write_response, AccessIpcReply,
    AccessIpcRequest, AccountChoice, ExecNoticeView, IpcRequest, IpcResponse,
};
use crate::share::access_approval::{ApprovalDecision, ApprovalId, ApprovalService, ApprovalView};
use crate::share::access_catalog::HelperStatus;
use crate::share::access_grant::{AccessGrantChange, AccessGrantView, CapabilityDecision};
use crate::share::exec_helper_protocol::AccountChoices;
use crate::share::{ExecGrantTarget, ExecId};

/// How long a client waits for an answer (`always` writes the journal).
const ACCESS_TIMEOUT: Duration = Duration::from_secs(20);
/// How often a client reads the notice of its waiting start.
const NOTICE_POLL: Duration = Duration::from_millis(500);
/// Failed reads in a row after which a client stops reading the notice
/// (a worker older than this client knows no `Access`).
const NOTICE_FAILURES: u32 = 20;

impl ShareHost {
    /// Serves one `IpcRequest::Access` whose daemon token was checked.
    pub(in crate::daemon) fn serve_access(
        &self,
        stream: &mut TcpStream,
        request: AccessIpcRequest,
    ) -> io::Result<()> {
        let response = match self.access_reply(request) {
            Ok(reply) => IpcResponse::Access(reply),
            Err(msg) => IpcResponse::Err { msg, code: None },
        };
        write_response(stream, &response)
    }

    fn access_reply(&self, request: AccessIpcRequest) -> Result<AccessIpcReply, String> {
        let approvals = crate::share::consent_launch::approvals();
        match request {
            AccessIpcRequest::MutateGrant {
                target,
                change,
                expected_revision,
            } => {
                let (result, grant) =
                    self.mutate_access_grant(target, &change, expected_revision)?;
                let own = crate::share::exec_provider_status().user_label;
                let now = crate::share::core_now_secs();
                let grant =
                    grant.map(|grant| AccessGrantView::new(&grant.access_grant(), &own, now));
                Ok(AccessIpcReply::GrantMutation { result, grant })
            }
            AccessIpcRequest::Approvals => Ok(AccessIpcReply::ApprovalList {
                pending: approvals.pending(),
            }),
            AccessIpcRequest::Decide {
                approval_id,
                decision,
            } => {
                let id = ApprovalId::parse(&approval_id)?;
                self.decide_approval(approvals, &id, decision)?;
                Ok(AccessIpcReply::Decided)
            }
            AccessIpcRequest::ReportUndeliverable { approval_id } => {
                approvals.report_undeliverable(&ApprovalId::parse(&approval_id)?)?;
                Ok(AccessIpcReply::Decided)
            }
            AccessIpcRequest::Accounts => Ok(self.account_list()),
            AccessIpcRequest::ExecNotice { exec_id } => Ok(AccessIpcReply::ExecNotice {
                notice: self.exec_notice(&exec_id),
            }),
        }
    }

    /// The answer to exactly the request `id`. „Immer erlauben“ first sets
    /// the capability to `allow`, compared against the revision the request
    /// was made under, then releases the waiting start.
    fn decide_approval(
        &self,
        approvals: &ApprovalService,
        id: &ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        if decision != ApprovalDecision::Always {
            return approvals.decide(id, decision);
        }
        let open = approvals
            .open(id)
            .ok_or_else(|| "the request is no longer open".to_string())?;
        let change = AccessGrantChange {
            capabilities: BTreeMap::from([(open.capability, CapabilityDecision::Allow)]),
            ..AccessGrantChange::default()
        };
        let target = open.grant_target()?;
        let (result, _) = self.mutate_access_grant(target, &change, Some(open.policy_revision))?;
        if !result.persisted || !result.applied || result.error.is_some() {
            let yes_no = |value: bool| if value { "yes" } else { "no" };
            return Err(format!(
                "\"always allow\" was not taken over completely (stored: {}, applied: {}): {}; \
                 the request stays open",
                yes_no(result.persisted),
                yes_no(result.applied),
                result
                    .error
                    .as_deref()
                    .unwrap_or("the worker has not applied it yet"),
            ));
        }
        // The waiting start reads the grant again. A request that ended
        // meanwhile still leaves the grant allowing for later starts.
        let _ = approvals.decide(id, ApprovalDecision::Always);
        Ok(())
    }

    /// "Ausführen als" of this worker (PRM-M6, PRM-M7, D.2.4): the state of
    /// its account helper and every account choice. The IPC client has a
    /// thread of its own (`ipc_listener.rs`), so asking the account helper
    /// here blocks no other request and no runtime.
    fn account_list(&self) -> AccessIpcReply {
        let AccountChoices { helper, choices } = crate::share::exec_account_choices();
        AccessIpcReply::AccountList { helper, choices }
    }

    fn exec_notice(&self, exec_id: &ExecId) -> Option<ExecNoticeView> {
        let notice = crate::share::ShareExecSession::notice_of(exec_id)?;
        let (active, _) = self.exec_state.snapshot();
        let device = active
            .into_iter()
            .find(|view| &view.exec_id == exec_id)
            .map(|view| view.peer_device_name)
            .unwrap_or_default();
        Some(ExecNoticeView {
            device,
            access: notice.access,
            approval: notice.approval,
        })
    }
}

/// One `Access` request to the running background worker; never starts one.
fn request(request: AccessIpcRequest) -> Result<AccessIpcReply, String> {
    let token = crate::daemon::ipc_storage::read_token()
        .map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = crate::daemon::ipc_storage::read_ipc_addr()
        .ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(ACCESS_TIMEOUT));
    write_request(&mut stream, &IpcRequest::Access { token, request })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::Access(reply) => Ok(reply),
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err(unexpected()),
    }
}

fn unexpected() -> String {
    "Unerwartete Worker-Antwort auf eine Zugriffsanfrage".into()
}

/// The open confirmations of this host; none while no worker runs.
pub(crate) fn access_approvals() -> Result<Vec<ApprovalView>, String> {
    if crate::daemon::ipc_storage::read_ipc_addr().is_none() {
        return Ok(Vec::new());
    }
    match request(AccessIpcRequest::Approvals)? {
        AccessIpcReply::ApprovalList { pending } => Ok(pending),
        _ => Err(unexpected()),
    }
}

/// The host user's answer to exactly the request `approval_id`.
pub(crate) fn decide_access_approval(
    approval_id: &str,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let approval_id = approval_id.to_string();
    match request(AccessIpcRequest::Decide {
        approval_id,
        decision,
    })? {
        AccessIpcReply::Decided => Ok(()),
        _ => Err(unexpected()),
    }
}

/// The app could not show the request `approval_id` (Android).
#[allow(dead_code)] // Only the Android facade (`mobile`) calls it; desktop builds lack it.
pub(crate) fn report_approval_undeliverable(approval_id: &str) -> Result<(), String> {
    let approval_id = approval_id.to_string();
    match request(AccessIpcRequest::ReportUndeliverable { approval_id })? {
        AccessIpcReply::Decided => Ok(()),
        _ => Err(unexpected()),
    }
}

/// What the host of this device's outgoing start `exec_id` told.
pub(crate) fn exec_notice(exec_id: &ExecId) -> Result<Option<ExecNoticeView>, String> {
    let exec_id = exec_id.clone();
    match request(AccessIpcRequest::ExecNotice { exec_id })? {
        AccessIpcReply::ExecNotice { notice } => Ok(notice),
        _ => Err(unexpected()),
    }
}

/// Changes one access grant (starts the worker when needed).
pub(crate) fn mutate_access_grant(
    target: ExecGrantTarget,
    change: AccessGrantChange,
    expected_revision: Option<u64>,
) -> Result<(ExecGrantPersistResult, Option<AccessGrantView>), String> {
    crate::daemon::ensure_worker_ready()?;
    match request(AccessIpcRequest::MutateGrant {
        target,
        change,
        expected_revision,
    })? {
        AccessIpcReply::GrantMutation { result, grant } => Ok((result, grant)),
        _ => Err(unexpected()),
    }
}

/// The account helper and the accounts a grant can name here, as the
/// running background worker sees them; never starts one. Without a worker
/// the caller asks `crate::share::exec_account_choices()` itself or shows
/// what it knows (completion, the access dialog).
pub(crate) fn access_accounts() -> Result<(HelperStatus, Vec<AccountChoice>), String> {
    if crate::daemon::ipc_storage::read_ipc_addr().is_none() {
        return Err("Background-Worker läuft nicht".into());
    }
    match request(AccessIpcRequest::Accounts)? {
        AccessIpcReply::AccountList { helper, choices } => Ok((helper, choices)),
        _ => Err(unexpected()),
    }
}

type NoticeSlot = Mutex<Option<ExecNoticeView>>;

/// Follows the notice of one outgoing start from a client process (`se
/// exec`, `se shell`, MCP): the worker is read every 500 ms until the last
/// clone of the watch goes.
#[derive(Clone)]
pub(crate) struct ExecNoticeWatch {
    latest: Arc<NoticeSlot>,
}

impl ExecNoticeWatch {
    /// `changed` runs on the watch's thread for every new notice.
    pub(crate) fn start(
        exec_id: ExecId,
        changed: impl FnMut(&ExecNoticeView) + Send + 'static,
    ) -> Self {
        let latest = Arc::new(NoticeSlot::default());
        let slot = Arc::downgrade(&latest);
        let _ = std::thread::Builder::new()
            .name("se-exec-notice".into())
            .spawn(move || follow(&exec_id, &slot, changed));
        Self { latest }
    }

    pub(crate) fn latest(&self) -> Option<ExecNoticeView> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The start waits for the other device's user.
    pub(crate) fn waiting(&self) -> bool {
        self.latest()
            .is_some_and(|notice| notice.approval.is_some())
    }
}

fn follow(exec_id: &ExecId, slot: &Weak<NoticeSlot>, mut changed: impl FnMut(&ExecNoticeView)) {
    let mut failures = 0;
    while let Some(latest) = slot.upgrade() {
        match exec_notice(exec_id) {
            Ok(Some(notice)) => {
                failures = 0;
                let fresh = {
                    let mut current = latest.lock().unwrap_or_else(PoisonError::into_inner);
                    let fresh = current.as_ref() != Some(&notice);
                    *current = Some(notice.clone());
                    fresh
                };
                if fresh {
                    changed(&notice);
                }
            }
            Ok(None) => failures = 0,
            Err(_) => {
                failures += 1;
                if failures >= NOTICE_FAILURES {
                    return;
                }
            }
        }
        drop(latest);
        std::thread::sleep(NOTICE_POLL);
    }
}
