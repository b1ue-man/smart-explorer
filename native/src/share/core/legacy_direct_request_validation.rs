use std::collections::HashSet;

use super::legacy_direct_request::{
    evidence_from_presence, legacy_selector, peer_from_presence, valid_hash_id,
    LegacyDirectDecisionSource, LegacyDirectDecisionState, LegacyDirectDeliveryState,
    LegacyDirectRequestEntry, MAX_LEGACY_DIRECT_REQUESTS, MAX_LEGACY_DIRECT_TOMBSTONES,
    MAX_LEGACY_PRESENCE_FUTURE_SECS,
};
use super::profiles::ShareProfiles;
use super::types::{DirectGrant, PeerPresence};

const MAX_ID_BYTES: usize = 256;
const MAX_NAME_BYTES: usize = 1024;
const MAX_PROOF_BYTES: usize = 512;
const MAX_CANDIDATES: usize = 32;
const MAX_CANDIDATE_BYTES: usize = 2048;
const MAX_TOTAL_CANDIDATE_BYTES: usize = 16 * 1024;
const MAX_ATTEMPT_ERROR_BYTES: usize = 2048;

impl ShareProfiles {
    pub(super) fn validate_legacy_direct_requests(&self) -> Result<(), String> {
        if self.legacy_direct_requests.len() > MAX_LEGACY_DIRECT_REQUESTS
            || self.legacy_direct_request_tombstones.len() > MAX_LEGACY_DIRECT_TOMBSTONES
        {
            return Err("legacy direct request ledger exceeds its bounded capacity".into());
        }
        let mut selectors = HashSet::new();
        let mut events = HashSet::new();
        for entry in &self.legacy_direct_requests {
            validate_entry(entry, self)?;
            if !selectors.insert(entry.selector.as_str())
                || !events.insert(entry.evidence.event_id.as_str())
            {
                return Err(format!("duplicate legacy request: {}", entry.selector));
            }
        }
        let mut tombstones = HashSet::new();
        for tombstone in &self.legacy_direct_request_tombstones {
            if !valid_hash_id(&tombstone.selector, "legacy-")
                || !valid_hash_id(&tombstone.event_id, "legacy-event-")
                || tombstone.deleted_at < 0
                || tombstone.retain_until < tombstone.deleted_at
                || events.contains(tombstone.event_id.as_str())
                || !tombstones.insert(tombstone.event_id.as_str())
            {
                return Err(format!(
                    "invalid deleted legacy request: {}",
                    tombstone.selector
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_legacy_evidence(
        &self,
        identity: &super::identity::ShareIdentity,
    ) -> Result<(), String> {
        for entry in &self.legacy_direct_requests {
            entry.verify_evidence(identity)?;
        }
        Ok(())
    }
}

pub(super) fn validate_presence(
    lookup_id: &str,
    presence: &PeerPresence,
    now: Option<i64>,
) -> Result<(), String> {
    validate_text("lookup_id", lookup_id, MAX_ID_BYTES, false)?;
    validate_text("device_name", &presence.device_name, MAX_NAME_BYTES, true)?;
    validate_text("nonce", &presence.nonce, MAX_ID_BYTES, false)?;
    validate_text("proof", &presence.proof, MAX_PROOF_BYTES, false)?;
    validate_text("relay_url", &presence.relay_url, MAX_CANDIDATE_BYTES, true)?;
    if presence.kind != "direct" || presence.relation_id != lookup_id {
        return Err("legacy presence has the wrong relation".into());
    }
    if presence.expires_at < 0 || now.is_some_and(|now| presence.expires_at < now) {
        return Err("legacy presence is expired".into());
    }
    if now.is_some_and(|now| {
        presence.expires_at > now.saturating_add(MAX_LEGACY_PRESENCE_FUTURE_SECS)
    }) {
        return Err("legacy presence expiry exceeds the accepted future window".into());
    }
    if presence.candidates.len() > MAX_CANDIDATES
        || presence
            .candidates
            .iter()
            .any(|value| value.len() > MAX_CANDIDATE_BYTES || has_control(value))
        || presence.candidates.iter().map(String::len).sum::<usize>() > MAX_TOTAL_CANDIDATE_BYTES
    {
        return Err("legacy presence candidates exceed the safe profile budget".into());
    }
    let peer = peer_from_presence(presence);
    peer.validate()
        .map_err(|error| format!("invalid legacy peer identity: {error}"))?;
    if presence.fingerprint != peer.fingerprint {
        return Err("legacy presence fingerprint does not match its public key".into());
    }
    Ok(())
}

pub(super) fn exact_grant(
    grant: &DirectGrant,
    peer: &super::direct_protocol::DirectPeerIdentity,
) -> bool {
    grant.device_id == peer.device_id
        && grant.public_key == peer.public_key
        && grant.node_id == peer.node_id
        && grant.fingerprint == peer.fingerprint
}

fn validate_entry(
    entry: &LegacyDirectRequestEntry,
    profiles: &ShareProfiles,
) -> Result<(), String> {
    let presence = entry.presence();
    validate_presence(&entry.lookup_id, &presence, None)?;
    let basic_invalid = entry.selector != legacy_selector(&entry.lookup_id, &entry.peer)
        || entry.evidence.event_id != evidence_from_presence(&entry.lookup_id, &presence).event_id
        || entry.first_received_at < 0
        || entry.last_received_at < entry.first_received_at
        || entry.last_received_at > entry.evidence.expires_at
        || entry.evidence.expires_at
            > entry
                .last_received_at
                .saturating_add(MAX_LEGACY_PRESENCE_FUTURE_SECS)
        || entry.decision_changed_at < entry.first_received_at
        || entry.decision_delivery.decision_revision > entry.decision_revision
        || entry
            .decision_delivery
            .last_attempt_at
            .is_some_and(|at| at < entry.decision_changed_at)
        || (entry.decision_delivery.attempt_count == 0)
            != entry.decision_delivery.last_attempt_at.is_none()
        || entry
            .decision_delivery
            .last_error
            .as_ref()
            .is_some_and(|error| error.len() > MAX_ATTEMPT_ERROR_BYTES || has_control(error));
    let accepted_without_exact_grant = entry.decision == LegacyDirectDecisionState::Accepted
        && !entry.authorization_active(profiles);
    if basic_invalid || accepted_without_exact_grant || !valid_state(entry) {
        return Err(format!("invalid legacy request: {}", entry.selector));
    }
    Ok(())
}

fn valid_state(entry: &LegacyDirectRequestEntry) -> bool {
    let delivery = &entry.decision_delivery;
    match entry.decision {
        LegacyDirectDecisionState::Pending => {
            entry.decision_source.is_none()
                && entry.decision_revision == 0
                && delivery.state == LegacyDirectDeliveryState::NotStarted
                && delivery.decision_revision == 0
                && delivery.attempt_count == 0
                && delivery.last_error.is_none()
        }
        LegacyDirectDecisionState::Accepted | LegacyDirectDecisionState::Rejected => {
            entry.decision_source.is_some()
                && entry.decision_revision > 0
                && delivery.decision_revision == entry.decision_revision
                && matches!(
                    delivery.state,
                    LegacyDirectDeliveryState::Queued
                        | LegacyDirectDeliveryState::AttemptedUntracked
                        | LegacyDirectDeliveryState::FailedUntracked
                )
                && (delivery.state == LegacyDirectDeliveryState::FailedUntracked)
                    == delivery.last_error.is_some()
        }
        LegacyDirectDecisionState::Revoked => {
            matches!(
                entry.decision_source,
                Some(
                    LegacyDirectDecisionSource::User
                        | LegacyDirectDecisionSource::AuthorizationLost
                )
            ) && entry.decision_revision > 0
                && delivery.state == LegacyDirectDeliveryState::LocalOnlyUntracked
                && delivery.decision_revision == entry.decision_revision
                && delivery.attempt_count == 0
                && delivery.last_error.is_none()
        }
        LegacyDirectDecisionState::Expired => {
            entry.decision_source.is_none()
                && entry.decision_revision > 0
                && delivery.state == LegacyDirectDeliveryState::NotStarted
                && delivery.decision_revision == 0
                && delivery.attempt_count == 0
                && delivery.last_error.is_none()
                && entry.decision_changed_at > entry.evidence.expires_at
        }
    }
}

fn validate_text(name: &str, value: &str, max: usize, empty: bool) -> Result<(), String> {
    if (!empty && value.is_empty()) || value.len() > max || has_control(value) {
        Err(format!("invalid legacy {name}"))
    } else {
        Ok(())
    }
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}
