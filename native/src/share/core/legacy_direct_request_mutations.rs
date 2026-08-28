use super::direct_protocol::DirectPeerIdentity;
use super::exec_policy::ExecGrant;
use super::legacy_direct_request::{
    evidence_from_presence, legacy_selector, peer_from_presence, LegacyDirectAnswer,
    LegacyDirectDecisionDelivery, LegacyDirectDecisionSource, LegacyDirectDecisionState,
    LegacyDirectDeliveryState, LegacyDirectRequestEntry, LegacyDirectRequestTombstone,
    MAX_LEGACY_DIRECT_REQUESTS, MAX_LEGACY_DIRECT_TOMBSTONES,
};
use super::legacy_direct_request_validation::{exact_grant, validate_presence};
use super::profiles::ShareProfiles;
use super::types::{DirectGrant, DirectGrantState, PeerPresence};

const MAX_ATTEMPT_ERROR_BYTES: usize = 2048;

impl ShareProfiles {
    pub fn legacy_direct_request(&self, selector: &str) -> Option<&LegacyDirectRequestEntry> {
        self.legacy_direct_requests
            .iter()
            .find(|entry| entry.selector == selector)
    }

    pub(crate) fn record_verified_legacy_direct_request(
        &mut self,
        lookup_id: &str,
        presence: &PeerPresence,
        now: i64,
    ) -> Result<bool, String> {
        validate_presence(lookup_id, presence, Some(now))?;
        let peer = peer_from_presence(presence);
        let selector = legacy_selector(lookup_id, &peer);
        let evidence = evidence_from_presence(lookup_id, presence);
        self.prune_legacy_tombstones(now);
        if self
            .legacy_direct_request_tombstones
            .iter()
            .any(|tombstone| {
                tombstone.event_id == evidence.event_id
                    || (tombstone.selector == selector && tombstone.retain_until >= now)
            })
        {
            return Ok(false);
        }
        let device_grant = self
            .direct_grants
            .iter()
            .find(|grant| grant.device_id == peer.device_id);
        let existing_grant = device_grant
            .filter(|grant| exact_grant(grant, &peer))
            .map(|grant| grant.state.clone());
        let identity_conflict = device_grant.is_some_and(|grant| !exact_grant(grant, &peer))
            || self.identity_conflicts(&peer, &selector);
        let policy_denied = self.direct_auto_accept_denied(lookup_id, &peer);
        if let Some(index) = self
            .legacy_direct_requests
            .iter()
            .position(|entry| entry.selector == selector)
        {
            let snapshot = self.legacy_direct_requests[index].clone();
            if snapshot.lookup_id != lookup_id
                || snapshot.peer.device_id != peer.device_id
                || snapshot.peer.public_key != peer.public_key
                || snapshot.peer.node_id != peer.node_id
            {
                return Err(format!("legacy request selector conflict: {selector}"));
            }
            if snapshot.evidence.event_id == evidence.event_id {
                return Ok(false);
            }
            let automatic = authenticated_decision(
                snapshot.decision,
                snapshot.decision_source,
                existing_grant,
                identity_conflict || policy_denied,
            );
            if automatic.install_grant {
                set_exact_grant(self, &peer, true, now)?;
            }
            let entry = &mut self.legacy_direct_requests[index];
            entry.peer.device_name = peer.device_name;
            entry.evidence = evidence;
            entry.last_received_at = now;
            entry.identity_conflict = identity_conflict;
            apply_authenticated_decision(entry, automatic, now);
            self.recompute_identity_conflicts_for_device(&peer.device_id);
            return Ok(true);
        }
        if self.legacy_direct_requests.len() >= MAX_LEGACY_DIRECT_REQUESTS {
            return Err(format!(
                "legacy request inbox is full (maximum {MAX_LEGACY_DIRECT_REQUESTS})"
            ));
        }
        let automatic = authenticated_decision(
            LegacyDirectDecisionState::Pending,
            None,
            existing_grant,
            identity_conflict || policy_denied,
        );
        if automatic.install_grant {
            set_exact_grant(self, &peer, true, now)?;
        }
        self.legacy_direct_requests.push(LegacyDirectRequestEntry {
            selector,
            lookup_id: lookup_id.to_string(),
            peer,
            evidence,
            first_received_at: now,
            last_received_at: now,
            decision: automatic.decision,
            decision_source: Some(automatic.source),
            decision_changed_at: now,
            decision_revision: 1,
            decision_delivery: queued_delivery(1),
            identity_conflict,
        });
        self.recompute_identity_conflicts_for_device(&presence.device_id);
        Ok(true)
    }

    pub(crate) fn decide_legacy_direct_request(
        &mut self,
        selector: &str,
        accepted: bool,
        now: i64,
    ) -> Result<(), String> {
        let index = self
            .legacy_direct_requests
            .iter()
            .position(|entry| entry.selector == selector)
            .ok_or_else(|| format!("legacy request not found: {selector}"))?;
        let snapshot = self.legacy_direct_requests[index].clone();
        if !snapshot.is_pending(now) {
            return Err(format!("legacy request is not pending: {selector}"));
        }
        let identity_conflict = self.identity_conflicts(&snapshot.peer, selector);
        if identity_conflict && accepted {
            return Err(format!(
                "legacy request has an identity conflict and cannot be accepted: {selector}"
            ));
        }
        if !identity_conflict {
            set_exact_grant(self, &snapshot.peer, accepted, now)?;
        }
        let entry = &mut self.legacy_direct_requests[index];
        entry.decision = if accepted {
            LegacyDirectDecisionState::Accepted
        } else {
            LegacyDirectDecisionState::Rejected
        };
        entry.decision_source = Some(LegacyDirectDecisionSource::User);
        entry.decision_changed_at = now;
        entry.decision_revision = entry.decision_revision.saturating_add(1).max(1);
        entry.decision_delivery = queued_delivery(entry.decision_revision);
        self.recompute_identity_conflicts_for_device(&snapshot.peer.device_id);
        Ok(())
    }

    pub(crate) fn revoke_legacy_direct_request(
        &mut self,
        selector: &str,
        now: i64,
    ) -> Result<(), String> {
        let index = self
            .legacy_direct_requests
            .iter()
            .position(|entry| entry.selector == selector)
            .ok_or_else(|| format!("legacy request not found: {selector}"))?;
        let snapshot = self.legacy_direct_requests[index].clone();
        if snapshot.decision != LegacyDirectDecisionState::Accepted {
            return Err(format!("legacy request is not accepted: {selector}"));
        }
        set_exact_grant(self, &snapshot.peer, false, now)?;
        let entry = &mut self.legacy_direct_requests[index];
        entry.decision = LegacyDirectDecisionState::Revoked;
        entry.decision_source = Some(LegacyDirectDecisionSource::User);
        entry.decision_changed_at = now;
        entry.decision_revision = entry.decision_revision.saturating_add(1);
        entry.decision_delivery = LegacyDirectDecisionDelivery {
            state: LegacyDirectDeliveryState::LocalOnlyUntracked,
            decision_revision: entry.decision_revision,
            ..Default::default()
        };
        Ok(())
    }

    pub(crate) fn delete_legacy_direct_request(
        &mut self,
        selector: &str,
        now: i64,
    ) -> Result<bool, String> {
        let Some(index) = self
            .legacy_direct_requests
            .iter()
            .position(|entry| entry.selector == selector)
        else {
            return Ok(false);
        };
        let entry = self.legacy_direct_requests[index].clone();
        if entry.authorization_active(self) {
            return Err(format!(
                "legacy request has an active authorization; revoke it before deletion: {selector}"
            ));
        }
        self.prune_legacy_tombstones(now);
        if self.legacy_direct_request_tombstones.len() >= MAX_LEGACY_DIRECT_TOMBSTONES {
            return Err(format!(
                "legacy request deletion ledger is full (maximum {MAX_LEGACY_DIRECT_TOMBSTONES})"
            ));
        }
        self.legacy_direct_request_tombstones
            .push(LegacyDirectRequestTombstone {
                selector: entry.selector,
                event_id: entry.evidence.event_id,
                deleted_at: now,
                retain_until: entry.evidence.expires_at.max(now),
            });
        self.legacy_direct_requests.remove(index);
        Ok(true)
    }

    pub fn legacy_answers_due(&self, now: i64) -> Vec<LegacyDirectAnswer> {
        self.legacy_direct_requests
            .iter()
            .filter(|entry| {
                entry.decision_delivery.state == LegacyDirectDeliveryState::Queued
                    || (entry.decision_delivery.state == LegacyDirectDeliveryState::FailedUntracked
                        && entry
                            .decision_delivery
                            .last_attempt_at
                            .is_none_or(|at| at.saturating_add(30) <= now))
            })
            .filter_map(|entry| {
                let accepted = match entry.decision {
                    LegacyDirectDecisionState::Accepted if entry.authorization_active(self) => true,
                    LegacyDirectDecisionState::Accepted => return None,
                    LegacyDirectDecisionState::Rejected => false,
                    _ => return None,
                };
                Some(LegacyDirectAnswer {
                    selector: entry.selector.clone(),
                    decision_revision: entry.decision_revision,
                    lookup_id: entry.lookup_id.clone(),
                    requester_device_id: entry.peer.device_id.clone(),
                    accepted,
                })
            })
            .collect()
    }

    pub(crate) fn record_legacy_answer_attempt(
        &mut self,
        selector: &str,
        decision_revision: u64,
        now: i64,
        error: Option<String>,
    ) -> Result<bool, String> {
        let Some(entry) = self
            .legacy_direct_requests
            .iter_mut()
            .find(|entry| entry.selector == selector)
        else {
            return Ok(false);
        };
        if entry.decision_revision != decision_revision {
            return Ok(false);
        }
        entry.decision_delivery.state = if error.is_some() {
            LegacyDirectDeliveryState::FailedUntracked
        } else {
            LegacyDirectDeliveryState::AttemptedUntracked
        };
        entry.decision_delivery.decision_revision = decision_revision;
        entry.decision_delivery.attempt_count =
            entry.decision_delivery.attempt_count.saturating_add(1);
        entry.decision_delivery.last_attempt_at = Some(now);
        entry.decision_delivery.last_error =
            error.map(|value| truncate(value, MAX_ATTEMPT_ERROR_BYTES));
        Ok(true)
    }

    pub(crate) fn retry_legacy_answer(&mut self, selector: &str) -> Result<(), String> {
        let entry = self
            .legacy_direct_requests
            .iter_mut()
            .find(|entry| entry.selector == selector)
            .ok_or_else(|| format!("legacy request not found: {selector}"))?;
        if !matches!(
            entry.decision,
            LegacyDirectDecisionState::Accepted | LegacyDirectDecisionState::Rejected
        ) || !matches!(
            entry.decision_delivery.state,
            LegacyDirectDeliveryState::AttemptedUntracked
                | LegacyDirectDeliveryState::FailedUntracked
        ) {
            return Err(format!(
                "legacy request has no retryable untracked answer: {selector}"
            ));
        }
        entry.decision_delivery.state = LegacyDirectDeliveryState::Queued;
        entry.decision_delivery.last_error = None;
        Ok(())
    }

    pub(crate) fn expire_legacy_direct_requests(&mut self, now: i64) -> usize {
        let mut changed = 0;
        for entry in &mut self.legacy_direct_requests {
            if entry.decision == LegacyDirectDecisionState::Pending
                && entry.evidence.expires_at < now
            {
                entry.decision = LegacyDirectDecisionState::Expired;
                entry.decision_changed_at = now;
                entry.decision_revision = entry.decision_revision.saturating_add(1);
                entry.decision_delivery = LegacyDirectDecisionDelivery::default();
                changed += 1;
            }
        }
        changed
    }
}

#[derive(Clone, Copy)]
struct AuthenticatedDecision {
    decision: LegacyDirectDecisionState,
    source: LegacyDirectDecisionSource,
    install_grant: bool,
}

fn authenticated_decision(
    previous: LegacyDirectDecisionState,
    previous_source: Option<LegacyDirectDecisionSource>,
    grant: Option<DirectGrantState>,
    identity_conflict: bool,
) -> AuthenticatedDecision {
    if identity_conflict {
        return AuthenticatedDecision {
            decision: LegacyDirectDecisionState::Rejected,
            source: previous_source
                .unwrap_or(LegacyDirectDecisionSource::AuthenticatedSecretPossession),
            install_grant: false,
        };
    }
    match grant {
        Some(DirectGrantState::Accepted) => AuthenticatedDecision {
            decision: LegacyDirectDecisionState::Accepted,
            source: LegacyDirectDecisionSource::ExistingGrant,
            install_grant: false,
        },
        Some(DirectGrantState::Ignored) => AuthenticatedDecision {
            decision: LegacyDirectDecisionState::Rejected,
            source: LegacyDirectDecisionSource::ExistingGrant,
            install_grant: false,
        },
        None
            if matches!(
                previous,
                LegacyDirectDecisionState::Rejected | LegacyDirectDecisionState::Revoked
            ) =>
        {
            AuthenticatedDecision {
                decision: LegacyDirectDecisionState::Rejected,
                source: previous_source
                    .unwrap_or(LegacyDirectDecisionSource::AuthenticatedSecretPossession),
                install_grant: false,
            }
        }
        None => AuthenticatedDecision {
            decision: LegacyDirectDecisionState::Accepted,
            source: LegacyDirectDecisionSource::AuthenticatedSecretPossession,
            install_grant: true,
        },
    }
}

fn apply_authenticated_decision(
    entry: &mut LegacyDirectRequestEntry,
    automatic: AuthenticatedDecision,
    now: i64,
) {
    if entry.decision == LegacyDirectDecisionState::Revoked
        && entry.decision_source == Some(LegacyDirectDecisionSource::User)
    {
        return;
    }
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

fn queued_delivery(revision: u64) -> LegacyDirectDecisionDelivery {
    LegacyDirectDecisionDelivery {
        state: LegacyDirectDeliveryState::Queued,
        decision_revision: revision,
        ..Default::default()
    }
}

fn set_exact_grant(
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
                return Ok(());
            }
            if grant.state == DirectGrantState::Accepted {
                return Err(format!(
                    "legacy peer identity conflicts with the active grant for device {}",
                    peer.device_id
                ));
            }
            grant.exec.reset_for_identity_change(now);
            grant.public_key = peer.public_key.clone();
            grant.fingerprint = peer.fingerprint.clone();
            grant.node_id = peer.node_id.clone();
        }
        if state != DirectGrantState::Accepted {
            grant.exec.disable_without_decision(now);
        }
        grant.device_name = peer.device_name.clone();
        grant.state = state;
        grant.updated_at = now;
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
    });
    Ok(())
}

fn truncate(mut value: String, max: usize) -> String {
    if value.len() <= max {
        return value;
    }
    let mut boundary = max;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}
