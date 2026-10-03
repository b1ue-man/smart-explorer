//! Automatic decisions for authenticated legacy Direct requests (FC5) and the
//! grant they install. A request from a device without a grant waits for the
//! user unless the device policy is `AutoAccept`.
use super::direct_protocol::DirectPeerIdentity;
use super::exec_policy::ExecGrant;
use super::legacy_direct_request::{
    LegacyDirectDecisionDelivery, LegacyDirectDecisionSource, LegacyDirectDecisionState,
    LegacyDirectDeliveryState, LegacyDirectRequestEntry,
};
use super::legacy_direct_request_validation::exact_grant;
use super::profiles::ShareProfiles;
use super::types::{DirectGrant, DirectGrantState, DirectRequestPolicy};

#[derive(Clone, Copy)]
pub(super) struct AuthenticatedDecision {
    pub(super) decision: LegacyDirectDecisionState,
    pub(super) source: LegacyDirectDecisionSource,
    pub(super) install_grant: bool,
}

/// Why an authenticated request is refused regardless of its grant.
#[derive(Clone, Copy)]
pub(super) struct Refusal {
    /// Competing identities claim the device.
    pub(super) identity_conflict: bool,
    /// The user removed or blocked this key, or rejected it before.
    pub(super) policy_denied: bool,
    /// A current explicit key/node denial wins over all active aliases.
    pub(super) key_denied: bool,
}

/// The decision this device takes without asking, or `None` when the request
/// waits for the user. The current grant of the exact identity wins over
/// older history; a suspended grant („neu bestätigen“) is confirmed again by
/// the request, which proves the current code.
pub(super) fn authenticated_decision(
    previous: LegacyDirectDecisionState,
    previous_source: Option<LegacyDirectDecisionSource>,
    grant: Option<DirectGrantState>,
    refusal: Refusal,
    policy: DirectRequestPolicy,
) -> Option<AuthenticatedDecision> {
    let rejected = |source| AuthenticatedDecision {
        decision: LegacyDirectDecisionState::Rejected,
        source,
        install_grant: false,
    };
    let kept_source =
        previous_source.unwrap_or(LegacyDirectDecisionSource::AuthenticatedSecretPossession);
    if refusal.identity_conflict || refusal.key_denied {
        return Some(rejected(kept_source));
    }
    match grant {
        Some(DirectGrantState::Accepted) => Some(AuthenticatedDecision {
            decision: LegacyDirectDecisionState::Accepted,
            source: LegacyDirectDecisionSource::ExistingGrant,
            install_grant: false,
        }),
        Some(DirectGrantState::Reconfirm) => Some(AuthenticatedDecision {
            decision: LegacyDirectDecisionState::Accepted,
            source: LegacyDirectDecisionSource::ExistingGrant,
            install_grant: true,
        }),
        Some(DirectGrantState::Ignored) => {
            Some(rejected(LegacyDirectDecisionSource::ExistingGrant))
        }
        None if matches!(
            previous,
            LegacyDirectDecisionState::Rejected | LegacyDirectDecisionState::Revoked
        ) || refusal.policy_denied =>
        {
            Some(rejected(kept_source))
        }
        None if policy == DirectRequestPolicy::AutoAccept => Some(AuthenticatedDecision {
            decision: LegacyDirectDecisionState::Accepted,
            source: LegacyDirectDecisionSource::AuthenticatedSecretPossession,
            install_grant: true,
        }),
        None => None,
    }
}

/// Applies an automatic decision to a re-received request. Without one, a
/// pending request stays pending and an expired one is pending again.
pub(super) fn apply_authenticated_decision(
    entry: &mut LegacyDirectRequestEntry,
    automatic: Option<AuthenticatedDecision>,
    now: i64,
) {
    if entry.decision == LegacyDirectDecisionState::Revoked
        && entry.decision_source == Some(LegacyDirectDecisionSource::User)
        && !automatic.is_some_and(|decision| {
            decision.decision == LegacyDirectDecisionState::Accepted
                && decision.source == LegacyDirectDecisionSource::ExistingGrant
        })
    {
        return;
    }
    let Some(automatic) = automatic else {
        if entry.decision == LegacyDirectDecisionState::Expired {
            reset_to_pending(entry, now);
        }
        return;
    };
    if entry.decision != automatic.decision {
        entry.decision = automatic.decision;
        entry.decision_source = Some(automatic.source);
        entry.decision_changed_at = now;
        entry.decision_revision = entry.decision_revision.saturating_add(1).max(1);
    } else {
        entry.decision_source.get_or_insert(automatic.source);
        entry.decision_revision = entry.decision_revision.max(1);
    }
    entry.decision_delivery = queued_delivery(entry.decision_revision);
}

/// The state of an undecided request (no source, no answer to deliver).
pub(super) fn reset_to_pending(entry: &mut LegacyDirectRequestEntry, now: i64) {
    entry.decision = LegacyDirectDecisionState::Pending;
    entry.decision_source = None;
    entry.decision_changed_at = now.max(entry.first_received_at);
    entry.decision_revision = 0;
    entry.decision_delivery = LegacyDirectDecisionDelivery::default();
}

pub(super) fn queued_delivery(revision: u64) -> LegacyDirectDecisionDelivery {
    LegacyDirectDecisionDelivery {
        state: LegacyDirectDeliveryState::Queued,
        decision_revision: revision,
        ..Default::default()
    }
}

impl ShareProfiles {
    /// Frees one place in a full legacy inbox: the oldest request nobody
    /// answered yet (pending or expired, no active authorization). A flood of
    /// requests with the Direct code (S28) thus only displaces itself, never
    /// decided history or authorized devices. Returns whether a place is free.
    pub(super) fn evict_unanswered_legacy_request(&mut self) -> bool {
        let Some(index) = self
            .legacy_direct_requests
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    entry.decision,
                    LegacyDirectDecisionState::Pending | LegacyDirectDecisionState::Expired
                ) && !entry.authorization_active(self)
            })
            .min_by_key(|(_, entry)| entry.last_received_at)
            .map(|(index, _)| index)
        else {
            return false;
        };
        self.legacy_direct_requests.remove(index);
        true
    }
}

pub(super) fn set_exact_grant(
    profiles: &mut ShareProfiles,
    peer: &DirectPeerIdentity,
    accepted: bool,
    now: i64,
) -> Result<(), String> {
    let state = if accepted {
        DirectGrantState::Accepted
    } else {
        DirectGrantState::Ignored
    };
    if let Some(grant) = profiles
        .direct_grants
        .iter_mut()
        .find(|grant| grant.device_id == peer.device_id)
    {
        if !exact_grant(grant, peer) {
            if !accepted {
                profiles.withdraw_direct_key(peer, now);
                return Ok(());
            }
            if grant.state == DirectGrantState::Accepted {
                return Err(format!(
                    "legacy peer identity conflicts with the active grant for device {}",
                    peer.device_id
                ));
            }
            grant.exec.reset_for_identity_change(now);
            grant.write = false;
            grant.public_key = peer.public_key.clone();
            grant.fingerprint = peer.fingerprint.clone();
            grant.node_id = peer.node_id.clone();
        }
        if state != DirectGrantState::Accepted || grant.state != DirectGrantState::Accepted {
            // Every change of the base authorization ends Exec (B04).
            grant.exec.disable_without_decision(now);
        }
        grant.device_name = peer.device_name.clone();
        grant.state = state;
        grant.updated_at = now;
        if !accepted {
            profiles.withdraw_direct_key(peer, now);
        }
        return Ok(());
    }
    profiles.direct_grants.push(DirectGrant {
        device_id: peer.device_id.clone(),
        device_name: peer.device_name.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
        state,
        updated_at: now,
        exec: ExecGrant::default(),
        write: false,
    });
    if !accepted {
        profiles.withdraw_direct_key(peer, now);
    }
    Ok(())
}

#[cfg(test)]
#[path = "legacy_direct_request_decision_task_tests.rs"]
mod task_tests;
