//! The decision about one Exec start (C1, C5, PRM-M3): the capability the
//! start needs is checked against the grant of that moment; on `ask` the host
//! asks its user through the approval service and keeps the client connected
//! meanwhile (`wait_for_approval`: heartbeats answered, early input kept, a
//! cancel ends the wait). Clients with `exec_approval_v1` learn where the host
//! asks (`APPROVAL_PENDING`). The grant is read again every `grant_poll`: a
//! narrowed or ended grant withdraws the request, one that allows now ends it
//! as allowed (`access_approval_answer.rs`). „Diesmal erlauben“ starts
//! exactly this start; after „Immer erlauben“ the daemon has set the
//! capability to `allow`, so `start_grant` passes. Every other end is a
//! refusal with its code, never a start. Each step is an access-log entry
//! (PRM-M4).
use std::io;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::AsyncWrite;

use super::super::{wait_for_approval, ApprovalWait, ClientStream};
use crate::share::access_approval::{
    ApprovalCommand, ApprovalOutcome, ApprovalService, ApprovalSummary, ReservationGauge,
};
use crate::share::access_audit::{AccessEntry, AccessEvent, ApprovalResult};
use crate::share::access_authorize::{capability_for, principal_grant, start_capability};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::exec_access_view::{wants_approval_frames, ExecAccessView, ExecApprovalPending};
use crate::share::exec_auth::AuthorizedExecPeer;
use crate::share::exec_client_messages::{
    approval_cancelled, approval_unavailable, permission_denied, right_not_granted,
    APPROVAL_CANCELLED, APPROVAL_UNAVAILABLE, PERMISSION_DENIED, RIGHT_NOT_GRANTED,
};
use crate::share::exec_heartbeat::ExecHeartbeatPolicy;
use crate::share::exec_protocol::{ExecWireError, ServerProtocolState};
use crate::share::exec_registry::ExecRegistryError;
use crate::share::exec_types::ExecAuthorization;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::types::ShareAuthState;
use crate::share::{ExecGrant, ExecStart};

// How a request ends: the answer, a change of the grant, a refusal.
#[path = "access_approval_answer.rs"]
mod answer;
#[cfg(test)]
use answer::grant_change;
use answer::{grant_watch, GrantChange, Log};

/// The grant at the start and what it decides for the start's capability.
pub(in crate::share) struct StartGrant {
    pub(in crate::share) grant: ExecGrant,
    /// The handshake's authorization at the grant's current revision: a
    /// grant that only widened since keeps the connection's authorization;
    /// a narrowing raised the registry's barrier above its epoch.
    pub(in crate::share) authorization: ExecAuthorization,
    pub(in crate::share) capability: Capability,
    /// Unix seconds at which the grant ends.
    pub(in crate::share) end: Option<i64>,
}

/// What the grant at the start decides for the start's capability.
pub(in crate::share) enum StartCheck {
    /// `allow`: the start goes ahead.
    Allowed(StartGrant),
    /// `ask`: the host user answers first.
    Ask(StartGrant),
}

/// `check_start` for a start that does not wait for the host user: `ask` is
/// refused as unavailable. After „Immer erlauben“ the grant allows, so the
/// start that waited passes here again.
pub(in crate::share) fn start_grant(
    auth: &Mutex<ShareAuthState>,
    authorized: &AuthorizedExecPeer,
    start: &ExecStart,
    host: &str,
    now: i64,
) -> Result<StartGrant, ExecWireError> {
    match check_start(auth, authorized, start, host, now)? {
        StartCheck::Allowed(decided) => Ok(decided),
        StartCheck::Ask(_) => Err(wire(APPROVAL_UNAVAILABLE, approval_unavailable(host))),
    }
}

/// Checks the capability `start` needs against the grant at `now`: `allow`
/// starts, `ask` waits for the host user, and `off` is `right_not_granted`
/// while the grant still allows the other Exec capability, else (ended or
/// withdrawn since the handshake) `permission_denied`.
pub(in crate::share) fn check_start(
    auth: &Mutex<ShareAuthState>,
    authorized: &AuthorizedExecPeer,
    start: &ExecStart,
    host: &str,
    now: i64,
) -> Result<StartCheck, ExecWireError> {
    let capability = start_capability(&start.command);
    let proven = ProvenPrincipal::from(&authorized.principal);
    let denied = || wire(PERMISSION_DENIED, permission_denied(host));
    let (decision, grant) = {
        let state = auth.lock().map_err(|_| denied())?;
        (
            capability_for(&state, &proven, capability, now),
            principal_grant(&state, &proven).cloned(),
        )
    };
    let Some(grant) = grant else {
        return Err(denied());
    };
    let decided = |grant: ExecGrant| StartGrant {
        end: grant.valid_until(),
        grant,
        authorization: ExecAuthorization {
            policy_revision: decision.policy_revision,
            ..authorized.authorization.clone()
        },
        capability,
    };
    match decision.decision {
        CapabilityDecision::Allow => Ok(StartCheck::Allowed(decided(grant))),
        CapabilityDecision::Ask => Ok(StartCheck::Ask(decided(grant))),
        CapabilityDecision::Off => {
            let other = Capability::EXEC
                .into_iter()
                .any(|other| other != capability && grant.decision_at(other, now).is_on());
            if other {
                Err(wire(RIGHT_NOT_GRANTED, right_not_granted(host, capability)))
            } else {
                Err(denied())
            }
        }
    }
}

/// Everything the decision about one start needs.
pub(in crate::share) struct StartApproval<'a> {
    pub(in crate::share) service: &'a ApprovalService,
    pub(in crate::share) gauge: &'a dyn ReservationGauge,
    pub(in crate::share) auth: &'a Mutex<ShareAuthState>,
    pub(in crate::share) authorized: &'a AuthorizedExecPeer,
    pub(in crate::share) start: &'a ExecStart,
    /// How refusals name this host.
    pub(in crate::share) host: &'a str,
    pub(in crate::share) client_features: &'a [String],
    /// The worker's account, as the access view names the own account.
    pub(in crate::share) own_label: &'a str,
    /// Where access-log entries go (`access_audit::record_access`).
    pub(in crate::share) audit: &'a (dyn Fn(AccessEntry<'_>) + Sync),
    /// How often the grant is read while the host user decides.
    pub(in crate::share) grant_poll: Duration,
}

/// A start that may go ahead, and how it was confirmed.
pub(in crate::share) struct ApprovedStart {
    pub(in crate::share) decided: StartGrant,
    pub(in crate::share) approval: ApprovalResult,
}

/// A start that does not go ahead.
pub(in crate::share) struct Refusal {
    pub(in crate::share) error: ExecWireError,
    /// Logged as `start_refused`; answers of the host user have entries of
    /// their own.
    pub(in crate::share) start_refused: bool,
}

impl Refusal {
    pub(in crate::share) fn start(error: ExecWireError) -> Self {
        Self {
            error,
            start_refused: true,
        }
    }

    fn answered(error: ExecWireError) -> Self {
        Self {
            error,
            start_refused: false,
        }
    }
}

/// The refusal of a start for which no reservation is free; the same as the
/// registry's own refusal at its limits.
pub(in crate::share) fn limit_error(outcome: ApprovalOutcome) -> ExecWireError {
    let limit = match outcome {
        ApprovalOutcome::HostLimit => ExecRegistryError::GlobalLimit,
        _ => ExecRegistryError::PrincipalLimit,
    };
    wire("admission_denied", limit.to_string())
}

/// The decision about one start: `allow` at once, `ask` with the host user.
pub(in crate::share) async fn decide_start<W: AsyncWrite + Unpin>(
    approval: &StartApproval<'_>,
    send: &mut W,
    stream: &mut ClientStream,
    protocol: &mut ServerProtocolState,
    policy: ExecHeartbeatPolicy,
) -> io::Result<Result<ApprovedStart, Refusal>> {
    let now = crate::share::core::now_secs();
    let checked = check_start(
        approval.auth,
        approval.authorized,
        approval.start,
        approval.host,
        now,
    );
    match checked {
        Ok(StartCheck::Allowed(decided)) => Ok(Ok(ApprovedStart {
            decided,
            approval: ApprovalResult::NotRequired,
        })),
        Ok(StartCheck::Ask(asked)) => ask(approval, asked, send, stream, protocol, policy).await,
        Err(error) => Ok(Err(Refusal::start(error))),
    }
}

enum Waited {
    Answer(ApprovalOutcome),
    Grant(GrantChange),
}

async fn ask<W: AsyncWrite + Unpin>(
    approval: &StartApproval<'_>,
    asked: StartGrant,
    send: &mut W,
    stream: &mut ClientStream,
    protocol: &mut ServerProtocolState,
    policy: ExecHeartbeatPolicy,
) -> io::Result<Result<ApprovedStart, Refusal>> {
    let principal = ProvenPrincipal::from(&approval.authorized.principal);
    let capability = asked.capability;
    let view = ExecAccessView::of(&asked.grant, approval.own_label);
    let summary = summary(approval, &principal, &asked, &view);
    let mut ticket = approval
        .service
        .request(approval.gauge, &principal, capability, summary);
    let log = Log {
        approval,
        principal: &principal,
        capability,
        id: ticket.id().clone(),
        view: &view,
    };
    let Some(surface) = ticket.surface() else {
        let outcome = ticket.outcome().await;
        return Ok(Err(log.refused(outcome)));
    };
    log.entry(AccessEvent::ApprovalRequested, None, None);
    let exec_id = &approval.start.exec_id;
    let pending = wants_approval_frames(approval.client_features)
        .then(|| ExecApprovalPending::new(exec_id.clone(), capability, surface));
    let waited = {
        let poll = approval.grant_poll;
        let grant = grant_watch(approval.auth, &principal, capability, &asked.grant, poll);
        let answer = ticket.outcome();
        let decision = async move {
            tokio::select! {
                outcome = answer => Waited::Answer(outcome),
                change = grant => Waited::Grant(change),
            }
        };
        wait_for_approval(send, stream, protocol, exec_id, pending, decision, policy).await
    };
    match waited {
        Ok(ApprovalWait::Decided(Waited::Answer(outcome))) => Ok(log.answered(outcome)),
        Ok(ApprovalWait::Decided(Waited::Grant(change))) => {
            drop(ticket);
            Ok(log.changed(change))
        }
        Ok(ApprovalWait::Cancelled) => {
            ticket.cancel();
            let reason = Some("the requesting device cancelled");
            log.entry(
                AccessEvent::ApprovalCancelled,
                Some(ApprovalResult::Cancelled),
                reason,
            );
            let error = wire(APPROVAL_CANCELLED, approval_cancelled(approval.host));
            Ok(Err(Refusal::answered(error)))
        }
        Err(error) => {
            // The ticket withdraws the request when it goes.
            drop(ticket);
            let reason = Some("the connection to the requesting device ended");
            log.entry(
                AccessEvent::ApprovalCancelled,
                Some(ApprovalResult::Cancelled),
                reason,
            );
            Err(error)
        }
    }
}

/// What the request shows of the start.
fn summary(
    approval: &StartApproval<'_>,
    principal: &ProvenPrincipal,
    asked: &StartGrant,
    view: &ExecAccessView,
) -> ApprovalSummary {
    let (device_name, room) = stored_names(approval.auth, principal);
    ApprovalSummary {
        device_name,
        room,
        account_label: Some(view.account_label.clone()),
        command: Some(ApprovalCommand::of(&approval.start.command)),
        policy_revision: asked.grant.policy_revision,
        older_client_terminal: asked.capability == Capability::ExecTerminal
            && !wants_approval_frames(approval.client_features),
    }
}

/// The names this host stored for the device and its room: the dialog
/// trusts them more than the name the device gives itself.
fn stored_names(
    auth: &Mutex<ShareAuthState>,
    principal: &ProvenPrincipal,
) -> (Option<String>, Option<String>) {
    let Ok(state) = auth.lock() else {
        return (None, None);
    };
    let same = |device_id: &str, public_key: &str| {
        device_id == principal.device_id && public_key == principal.public_key
    };
    match principal.relation_kind.as_str() {
        "direct" => {
            let grant = state
                .direct_grants
                .iter()
                .find(|grant| same(&grant.device_id, &grant.public_key));
            (grant.map(|grant| grant.device_name.clone()), None)
        }
        "room" => {
            let room = state
                .rooms
                .iter()
                .find(|room| room.room_id == principal.relation_id);
            let member = room.and_then(|room| {
                room.members
                    .iter()
                    .find(|member| same(&member.device_id, &member.public_key))
            });
            (
                member.map(|member| member.device_name.clone()),
                room.map(|room| room.name.clone()),
            )
        }
        _ => (None, None),
    }
}

fn wire(code: &str, message: String) -> ExecWireError {
    ExecWireError {
        code: code.into(),
        message,
    }
}

#[cfg(test)]
#[path = "access_approval_tests.rs"]
mod tests;
