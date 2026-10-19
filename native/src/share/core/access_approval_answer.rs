//! How one confirmation of an Exec start ends (PRM-M3, C5): the host
//! user's answer, a change of the grant while the request is open (read again
//! every `grant_poll`), or a refusal of the request itself. „Diesmal
//! erlauben“ starts exactly this start under the grant of that moment; after
//! „Immer erlauben“ the daemon has set the capability to `allow`, so
//! `start_grant` passes; a grant that allows now ends the request as allowed;
//! a narrowed or ended grant withdraws it. Every end is an access-log entry
//! (PRM-M4); none of them allows by default.
use std::sync::Mutex;
use std::time::Duration;

use super::{check_start, start_grant, wire, ApprovedStart, Refusal};
use super::{StartApproval, StartCheck};
use crate::share::access_approval::{ApprovalId, ApprovalOutcome};
use crate::share::access_audit::{AccessEntry, AccessEvent, AccessKind, ApprovalResult};
use crate::share::access_authorize::principal_grant;
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::exec_access_view::ExecAccessView;
use crate::share::exec_client_messages::{
    approval_cancelled, approval_denied, approval_unavailable, approval_unavailable_notifications,
    APPROVAL_CANCELLED, APPROVAL_DENIED, APPROVAL_UNAVAILABLE,
};
use crate::share::exec_protocol::ExecWireError;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::types::ShareAuthState;
use crate::share::ExecGrant;

/// How the grant changed while its request was open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GrantChange {
    /// The capability is `allow` now (for instance after „Immer erlauben“
    /// on another request of the device).
    Allows,
    /// Narrowed (C5): off, another account, another program list, an
    /// earlier end.
    Narrowed,
    /// Ended, or the grant or its relation is gone.
    Ended,
}

/// Resolves when the grant of `principal` no longer asks for `capability`
/// as it did for `asked`.
pub(super) async fn grant_watch(
    auth: &Mutex<ShareAuthState>,
    principal: &ProvenPrincipal,
    capability: Capability,
    asked: &ExecGrant,
    poll: Duration,
) -> GrantChange {
    loop {
        tokio::time::sleep(poll).await;
        let now = crate::share::core::now_secs();
        if let Some(change) = grant_change(auth, principal, capability, asked, now) {
            return change;
        }
    }
}

/// How the grant changed against `asked`; `None` while it asks as before.
pub(super) fn grant_change(
    auth: &Mutex<ShareAuthState>,
    principal: &ProvenPrincipal,
    capability: Capability,
    asked: &ExecGrant,
    now: i64,
) -> Option<GrantChange> {
    let Ok(state) = auth.lock() else {
        return Some(GrantChange::Ended);
    };
    let Some(grant) = principal_grant(&state, principal) else {
        return Some(GrantChange::Ended);
    };
    match grant.decision_at(capability, now) {
        CapabilityDecision::Allow => Some(GrantChange::Allows),
        CapabilityDecision::Off if grant.access_grant().expired_at(now) => Some(GrantChange::Ended),
        CapabilityDecision::Off => Some(GrantChange::Narrowed),
        CapabilityDecision::Ask => grant
            .narrowed_against(asked)
            .contains(&capability)
            .then_some(GrantChange::Narrowed),
    }
}

/// The access-log entries of one request and the ends they log.
pub(super) struct Log<'a, 'b> {
    pub(super) approval: &'a StartApproval<'b>,
    pub(super) principal: &'a ProvenPrincipal,
    pub(super) capability: Capability,
    pub(super) id: ApprovalId,
    pub(super) view: &'a ExecAccessView,
}

impl Log<'_, '_> {
    pub(super) fn entry(
        &self,
        event: AccessEvent,
        result: Option<ApprovalResult>,
        reason: Option<&str>,
    ) {
        (self.approval.audit)(AccessEntry {
            kind: AccessKind::Exec {
                start: Some(self.approval.start),
            },
            principal: self.principal,
            outcome: Some(event),
            capability: self.capability,
            approval: result,
            approval_id: Some(self.id.as_str()),
            account_profile: Some(&self.view.account_profile),
            account: Some(&self.view.account_label),
            reason,
        });
    }

    fn check(&self) -> Result<StartCheck, ExecWireError> {
        let approval = self.approval;
        let now = crate::share::core::now_secs();
        check_start(
            approval.auth,
            approval.authorized,
            approval.start,
            approval.host,
            now,
        )
    }

    /// The host user answered.
    pub(super) fn answered(&self, outcome: ApprovalOutcome) -> Result<ApprovedStart, Refusal> {
        let result = match outcome {
            ApprovalOutcome::Once => ApprovalResult::Once,
            ApprovalOutcome::Always => ApprovalResult::Always,
            refused => return Err(self.refused(refused)),
        };
        self.entry(AccessEvent::ApprovalGranted, Some(result), None);
        let approval = self.approval;
        let now = crate::share::core::now_secs();
        let decided = match outcome {
            // „Immer erlauben“: the grant allows now, like for every later
            // start.
            ApprovalOutcome::Always => {
                start_grant(
                    approval.auth,
                    approval.authorized,
                    approval.start,
                    approval.host,
                    now,
                )
                .map(StartCheck::Allowed)
                // The change has not reached this runtime yet; the answer
                // still allows this one start.
                .or_else(|error| match error.code.as_str() {
                    APPROVAL_UNAVAILABLE => self.check(),
                    _ => Err(error),
                })
            }
            // „Diesmal erlauben“: exactly this start, under the grant of now.
            _ => self.check(),
        };
        match decided {
            Ok(StartCheck::Allowed(decided) | StartCheck::Ask(decided)) => Ok(ApprovedStart {
                decided,
                approval: result,
            }),
            Err(error) => Err(Refusal::start(error)),
        }
    }

    /// The grant changed while the request was open.
    pub(super) fn changed(&self, change: GrantChange) -> Result<ApprovedStart, Refusal> {
        if change == GrantChange::Allows {
            if let Ok(StartCheck::Allowed(decided)) = self.check() {
                let reason = Some("the grant allows it now");
                self.entry(
                    AccessEvent::ApprovalGranted,
                    Some(ApprovalResult::Always),
                    reason,
                );
                return Ok(ApprovedStart {
                    decided,
                    approval: ApprovalResult::Always,
                });
            }
        }
        // Withdrawn: refused as the grant decides now, else as withdrawn.
        let host = self.approval.host;
        let error = match self.check() {
            Err(error) => error,
            Ok(_) => wire(APPROVAL_CANCELLED, approval_cancelled(host)),
        };
        let (event, reason) = match change {
            GrantChange::Ended => (AccessEvent::GrantExpired, "the grant ended"),
            _ => (AccessEvent::ApprovalCancelled, "the grant was narrowed"),
        };
        self.entry(event, Some(ApprovalResult::Cancelled), Some(reason));
        Err(Refusal::answered(error))
    }

    /// A request that ended without the host user allowing it.
    pub(super) fn refused(&self, outcome: ApprovalOutcome) -> Refusal {
        let host = self.approval.host;
        let (event, result, reason, error) = match outcome {
            ApprovalOutcome::Denied => (
                AccessEvent::ApprovalDenied,
                ApprovalResult::Denied,
                None,
                wire(APPROVAL_DENIED, approval_denied(host)),
            ),
            ApprovalOutcome::Withdrawn => (
                AccessEvent::ApprovalCancelled,
                ApprovalResult::Cancelled,
                Some("withdrawn on this host"),
                wire(APPROVAL_CANCELLED, approval_cancelled(host)),
            ),
            ApprovalOutcome::Undeliverable => (
                AccessEvent::ApprovalUnavailable,
                ApprovalResult::Unavailable,
                Some("notifications for Smart Explorer are off"),
                wire(
                    APPROVAL_UNAVAILABLE,
                    approval_unavailable_notifications(host),
                ),
            ),
            // `Unavailable`; an answer that allows never reaches here.
            _ => (
                AccessEvent::ApprovalUnavailable,
                ApprovalResult::Unavailable,
                Some("nobody on this host can be asked"),
                wire(APPROVAL_UNAVAILABLE, approval_unavailable(host)),
            ),
        };
        self.entry(event, Some(result), reason);
        Refusal::answered(error)
    }
}
