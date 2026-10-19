//! Rights of one screen session (decision C7, contract V7): after
//! `prove_relation` (share host) the host reads `capability_for(…, now)` of
//! `ScreenView` and `ScreenControl` through `ScreenAuthority`, decides the
//! session, asks its user through the consent broker on „Fragen“ and logs
//! every step with `record_access` (`AccessKind::Screen { mode, event }`).
//! A probe and a tile preview never ask; under „Fragen“ they get the facts
//! without a picture (C6). „Diesmal erlauben“ allows exactly this session,
//! „Immer erlauben“ has set the capability to `allow` (daemon `Decide`);
//! every other end refuses. No answer has a time limit (C5).
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use super::session_state::EndReason;
use super::wire_hello::{ApprovalKind, ScreenErrorCode, SessionMode};
use crate::share::access_approval::{ApprovalOutcome, ApprovalSummary};
use crate::share::access_audit::{AccessEntry, AccessEvent, AccessKind, ApprovalResult};
use crate::share::access_grant::{Capability, CapabilityDecision, CapabilityState};
use crate::share::relation_proof::ProvenPrincipal;

use super::host_backend::ConsentBroker;

/// How often a session that waits for its host user reads its right again.
pub(crate) const GRANT_POLL: Duration = Duration::from_secs(1);

/// Where access-log entries go: `record_access`; tests capture them.
pub(crate) type AuditSink = fn(AccessEntry<'_>);

/// The rights of proven devices as the share host keeps them.
pub(crate) trait ScreenAuthority: Send + Sync {
    /// The decision of `capability` for `principal` at `now`; `None` when
    /// the principal has no grant in an active relation.
    fn capability(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        now: i64,
    ) -> Option<CapabilityState>;

    /// Runs `admit` while no change of rights can interleave, if every
    /// capability of `needed` is not off for `principal` at `now`.
    fn admit(
        &self,
        principal: &ProvenPrincipal,
        needed: &[Capability],
        now: i64,
        admit: &mut dyn FnMut(),
    ) -> bool;

    /// The names this host stored for the device and its room.
    fn stored_names(&self, principal: &ProvenPrincipal) -> (Option<String>, Option<String>);
}

/// What the host does with a session request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionDecision {
    /// Rights, platform, codecs and limits only: a probe, or a preview
    /// under „Fragen“ (it asks only when raised to the full view).
    InfoOnly,
    Allowed,
    /// The host user answers first, for this capability.
    Ask(Capability),
    Refused(ScreenErrorCode),
}

/// The decision for `mode` under `view` and `control` (C7): without „ansehen“
/// nothing (`permission_denied`, no detail for strangers); control needs
/// both (`right_not_granted` when only viewing is allowed).
pub(crate) fn decide(
    mode: SessionMode,
    view: CapabilityDecision,
    control: CapabilityDecision,
) -> SessionDecision {
    use CapabilityDecision::{Allow, Ask, Off};
    if view == Off {
        return SessionDecision::Refused(ScreenErrorCode::PermissionDenied);
    }
    match mode {
        SessionMode::Probe => SessionDecision::InfoOnly,
        // A tile preview never asks (C6): under „Fragen“ it learns the rights.
        _ if view == Ask && !mode.may_ask() => SessionDecision::InfoOnly,
        SessionMode::Preview => SessionDecision::Allowed,
        SessionMode::View if view == Ask => SessionDecision::Ask(Capability::ScreenView),
        SessionMode::View => SessionDecision::Allowed,
        SessionMode::Control => match (view, control) {
            (_, Off) => SessionDecision::Refused(ScreenErrorCode::RightNotGranted),
            (_, Ask) => SessionDecision::Ask(Capability::ScreenControl),
            (Ask, Allow) => SessionDecision::Ask(Capability::ScreenView),
            (_, Allow) => SessionDecision::Allowed,
        },
    }
}

/// The capability an entry about `mode` names.
pub(crate) fn mode_capability(mode: SessionMode) -> Capability {
    match mode {
        SessionMode::Control => Capability::ScreenControl,
        _ => Capability::ScreenView,
    }
}

/// Logs one screen-session event (`session_started`, `control_taken`,
/// `control_released`, `session_ended`, `session_refused`).
pub(crate) fn log_event(
    audit: AuditSink,
    principal: &ProvenPrincipal,
    mode: SessionMode,
    event: &str,
    approval: Option<ApprovalResult>,
    reason: Option<&str>,
) {
    audit(AccessEntry {
        kind: AccessKind::Screen {
            mode: mode.key(),
            event,
        },
        principal,
        outcome: None,
        capability: mode_capability(mode),
        approval,
        approval_id: None,
        account_profile: None,
        account: None,
        reason,
    });
}

/// Logs one step of a confirmation (`approval_*`, `grant_expired`).
pub(crate) fn log_outcome(
    audit: AuditSink,
    principal: &ProvenPrincipal,
    mode: SessionMode,
    capability: Capability,
    outcome: AccessEvent,
    approval: (Option<ApprovalResult>, Option<&str>),
    reason: Option<&str>,
) {
    let (result, approval_id) = approval;
    audit(AccessEntry {
        kind: AccessKind::Screen {
            mode: mode.key(),
            event: outcome.key(),
        },
        principal,
        outcome: Some(outcome),
        capability,
        approval: result,
        approval_id,
        account_profile: None,
        account: None,
        reason,
    });
}

/// How a confirmation ended for the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Approval {
    Granted(ApprovalKind),
    Refused(ScreenErrorCode),
    /// The session was ended while it waited (narrowed, stopped).
    Ended(EndReason),
}

/// Everything one confirmation needs.
pub(crate) struct AskContext<'a> {
    pub authority: &'a dyn ScreenAuthority,
    pub broker: &'a Arc<dyn ConsentBroker>,
    pub audit: AuditSink,
    pub principal: &'a ProvenPrincipal,
    pub mode: SessionMode,
    pub capability: Capability,
    pub policy_revision: u64,
}

/// What the request shows of the session (V7 `summary`).
fn summary(context: &AskContext<'_>) -> ApprovalSummary {
    let (device_name, room) = context.authority.stored_names(context.principal);
    ApprovalSummary {
        device_name,
        room,
        account_label: None,
        command: None,
        policy_revision: context.policy_revision,
        older_client_terminal: false,
    }
}

/// Asks the host user; `on_shown` is awaited once the request is shown
/// (the session sends `APPROVAL_PENDING` there). `viewer_gone` resolves when
/// the viewer withdraws (closes); `stop` turns `Some` when the session is
/// ended meanwhile. The right is read again every `GRANT_POLL`: one that
/// allows now ends the wait as allowed, one that is off refuses.
pub(crate) async fn ask(
    context: AskContext<'_>,
    on_shown: impl std::future::Future<Output = ()>,
    viewer_gone: impl std::future::Future<Output = ()>,
    mut stop: watch::Receiver<Option<EndReason>>,
) -> Approval {
    let (principal, mode, capability) = (context.principal, context.mode, context.capability);
    if let Some(reason) = *stop.borrow() {
        return Approval::Ended(reason);
    }
    let audit = context.audit;
    let mut ticket = context
        .broker
        .request(principal, capability, summary(&context));
    let id = ticket.id().as_str().to_string();
    let log = |event: AccessEvent, result: Option<ApprovalResult>, reason: Option<&str>| {
        log_outcome(
            audit,
            principal,
            mode,
            capability,
            event,
            (result, Some(&id)),
            reason,
        );
    };
    if ticket.surface().is_none() {
        let outcome = ticket.outcome().await;
        return refused(outcome, &log);
    }
    log(AccessEvent::ApprovalRequested, None, None);
    on_shown.await;
    tokio::pin!(viewer_gone);
    let mut poll = tokio::time::interval(GRANT_POLL);
    poll.tick().await;
    loop {
        tokio::select! {
            outcome = ticket.outcome() => {
                return match outcome {
                    ApprovalOutcome::Once => {
                        log(AccessEvent::ApprovalGranted, Some(ApprovalResult::Once), None);
                        Approval::Granted(ApprovalKind::Once)
                    }
                    ApprovalOutcome::Always => {
                        log(AccessEvent::ApprovalGranted, Some(ApprovalResult::Always), None);
                        Approval::Granted(ApprovalKind::Always)
                    }
                    other => refused(other, &log),
                };
            }
            () = &mut viewer_gone => {
                ticket.cancel();
                let reason = Some("the requesting device cancelled");
                log(AccessEvent::ApprovalCancelled, Some(ApprovalResult::Cancelled), reason);
                return Approval::Refused(ScreenErrorCode::ApprovalCancelled);
            }
            changed = stop.changed() => {
                let signalled = *stop.borrow();
                if changed.is_err() || signalled.is_some() {
                    let reason = signalled.unwrap_or(EndReason::HostShutdown);
                    ticket.cancel();
                    let cancelled = Some(ApprovalResult::Cancelled);
                    log(AccessEvent::ApprovalCancelled, cancelled, Some(reason.key()));
                    return Approval::Ended(reason);
                }
            }
            _ = poll.tick() => {
                let now = crate::share::core_now_secs();
                let decision = context
                    .authority
                    .capability(principal, capability, now)
                    .map(|state| state.decision);
                match decision {
                    Some(CapabilityDecision::Allow) => {
                        drop(ticket);
                        let reason = Some("the grant allows it now");
                        log(AccessEvent::ApprovalGranted, Some(ApprovalResult::Always), reason);
                        return Approval::Granted(ApprovalKind::Always);
                    }
                    Some(CapabilityDecision::Ask) => {}
                    _ => {
                        drop(ticket);
                        let reason = Some("the grant was narrowed");
                        let cancelled = Some(ApprovalResult::Cancelled);
                        log(AccessEvent::ApprovalCancelled, cancelled, reason);
                        return Approval::Refused(ScreenErrorCode::PermissionDenied);
                    }
                }
            }
        }
    }
}

/// A request that ended without the host user allowing it.
fn refused(
    outcome: ApprovalOutcome,
    log: &dyn Fn(AccessEvent, Option<ApprovalResult>, Option<&str>),
) -> Approval {
    let (event, result, reason, code) = match outcome {
        ApprovalOutcome::Denied => (
            AccessEvent::ApprovalDenied,
            ApprovalResult::Denied,
            None,
            ScreenErrorCode::ApprovalDenied,
        ),
        ApprovalOutcome::Withdrawn => (
            AccessEvent::ApprovalCancelled,
            ApprovalResult::Cancelled,
            Some("withdrawn on this host"),
            ScreenErrorCode::ApprovalCancelled,
        ),
        ApprovalOutcome::Undeliverable => (
            AccessEvent::ApprovalUnavailable,
            ApprovalResult::Unavailable,
            Some("notifications for Smart Explorer are off"),
            ScreenErrorCode::ApprovalUnavailable,
        ),
        // `Unavailable`; an answer that allows never reaches here.
        _ => (
            AccessEvent::ApprovalUnavailable,
            ApprovalResult::Unavailable,
            Some("nobody on this host can be asked"),
            ScreenErrorCode::ApprovalUnavailable,
        ),
    };
    log(event, Some(result), reason);
    Approval::Refused(code)
}

#[cfg(test)]
#[path = "authorize_tests.rs"]
mod tests;
