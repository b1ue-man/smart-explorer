use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::core::{hex, presence_payload, public_fingerprint, verify_hmac};
use super::direct_ledger::DirectRequestDirection;
use super::direct_protocol::DirectPeerIdentity;
use super::identity::ShareIdentity;
use super::profiles::ShareProfiles;
use super::types::{DirectAccessState, DirectGrantState, PeerPresence};

pub const MAX_LEGACY_DIRECT_REQUESTS: usize = 24;
pub const MAX_LEGACY_PRESENCE_FUTURE_SECS: i64 = super::types::MAX_PRESENCE_FUTURE_SECS;
pub(crate) const MAX_LEGACY_DIRECT_TOMBSTONES: usize = 64;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyDirectDecisionState {
    Pending,
    Accepted,
    Rejected,
    Revoked,
    Expired,
}

impl LegacyDirectDecisionState {
    pub fn code(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyDirectDecisionSource {
    User,
    ExistingGrant,
    AuthenticatedSecretPossession,
    AuthorizationLost,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyDirectDeliveryState {
    #[default]
    NotStarted,
    Queued,
    AttemptedUntracked,
    FailedUntracked,
    LocalOnlyUntracked,
}

impl LegacyDirectDeliveryState {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Queued => "queued",
            Self::AttemptedUntracked => "attempted_untracked",
            Self::FailedUntracked => "failed_untracked",
            Self::LocalOnlyUntracked => "local_only_untracked",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyDirectDecisionDelivery {
    pub state: LegacyDirectDeliveryState,
    pub decision_revision: u64,
    pub attempt_count: u32,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyDirectPresenceEvidence {
    pub event_id: String,
    pub relay_url: String,
    pub candidates: Vec<String>,
    pub expires_at: i64,
    pub nonce: String,
    pub proof: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyDirectRequestEntry {
    pub selector: String,
    pub lookup_id: String,
    pub peer: DirectPeerIdentity,
    pub evidence: LegacyDirectPresenceEvidence,
    pub first_received_at: i64,
    pub last_received_at: i64,
    pub decision: LegacyDirectDecisionState,
    pub decision_source: Option<LegacyDirectDecisionSource>,
    pub decision_changed_at: i64,
    pub decision_revision: u64,
    pub decision_delivery: LegacyDirectDecisionDelivery,
    #[serde(default)]
    pub identity_conflict: bool,
}

impl LegacyDirectRequestEntry {
    pub fn is_pending(&self, now: i64) -> bool {
        self.decision == LegacyDirectDecisionState::Pending && self.evidence.expires_at >= now
    }

    pub fn authorization_active(&self, profiles: &ShareProfiles) -> bool {
        profiles.direct_grants.iter().any(|grant| {
            super::legacy_direct_request_validation::exact_grant(grant, &self.peer)
                && grant.state == DirectGrantState::Accepted
        })
    }

    pub fn decision_delivery_channel(&self) -> &'static str {
        match self.decision {
            LegacyDirectDecisionState::Accepted | LegacyDirectDecisionState::Rejected => {
                "legacy_signaling_untracked"
            }
            LegacyDirectDecisionState::Revoked => "local_only_untracked",
            LegacyDirectDecisionState::Pending | LegacyDirectDecisionState::Expired => {
                "not_applicable"
            }
        }
    }

    pub fn presence(&self) -> PeerPresence {
        PeerPresence {
            kind: "direct".into(),
            relation_id: self.lookup_id.clone(),
            device_id: self.peer.device_id.clone(),
            device_name: self.peer.device_name.clone(),
            public_key: self.peer.public_key.clone(),
            fingerprint: self.peer.fingerprint.clone(),
            node_id: self.peer.node_id.clone(),
            relay_url: self.evidence.relay_url.clone(),
            candidates: self.evidence.candidates.clone(),
            expires_at: self.evidence.expires_at,
            nonce: self.evidence.nonce.clone(),
            proof: self.evidence.proof.clone(),
        }
    }

    pub fn verify_for_local_identity(
        &self,
        identity: &ShareIdentity,
        now: i64,
    ) -> Result<(), String> {
        if self.lookup_id != identity.direct_lookup_id {
            return Err("legacy request belongs to a different local direct identity".into());
        }
        if self.evidence.expires_at < now {
            return Err(format!("legacy request is expired: {}", self.selector));
        }
        self.verify_evidence(identity)
    }

    pub(crate) fn verify_evidence(&self, identity: &ShareIdentity) -> Result<(), String> {
        if self.lookup_id != identity.direct_lookup_id {
            return Err("legacy request belongs to a different local direct identity".into());
        }
        let presence = self.presence();
        super::legacy_direct_request_validation::validate_presence(
            &self.lookup_id,
            &presence,
            None,
        )?;
        let payload = presence_payload(
            "direct",
            &self.lookup_id,
            &presence.device_id,
            &presence.public_key,
            &presence.node_id,
            &presence.relay_url,
            &presence.candidates,
            presence.expires_at,
            &presence.nonce,
        );
        if !verify_hmac(&identity.direct_secret(), &payload, &presence.proof) {
            return Err(format!(
                "legacy request authentication no longer verifies: {}",
                self.selector
            ));
        }
        Ok(())
    }
}

impl ShareProfiles {
    /// Returns whether durable, exact-identity policy forbids automatic
    /// acceptance. Authentication proves secret possession; it does not erase
    /// an ignored contact or a user's retained rejection/revocation history.
    pub(crate) fn direct_auto_accept_denied(
        &self,
        lookup_id: &str,
        peer: &DirectPeerIdentity,
    ) -> bool {
        let ignored_grant = self.direct_grants.iter().any(|grant| {
            grant.state == DirectGrantState::Ignored
                && grant.device_id == peer.device_id
                && grant.public_key == peer.public_key
                && grant.fingerprint == peer.fingerprint
                && grant.node_id == peer.node_id
        });
        let ignored_contact = self.direct_contacts.iter().any(|contact| {
            contact.access_state == DirectAccessState::Ignored
                && contact.remote_device_id.as_deref() == Some(peer.device_id.as_str())
                && contact.remote_public_key.as_deref() == Some(peer.public_key.as_str())
                && contact.expected_fingerprint == peer.fingerprint
                && (contact.expected_node_id.is_empty()
                    || contact.expected_node_id == peer.node_id)
        });
        let tracked_tombstone = self.direct_request_tombstones.iter().any(|tombstone| {
            tombstone.direction == DirectRequestDirection::Incoming
                && tombstone.request.lookup_id == lookup_id
                && tombstone.request.requester == *peer
        });
        let selector = legacy_selector(lookup_id, peer);
        let user_denial = self.legacy_direct_requests.iter().any(|entry| {
            entry.selector == selector
                && entry.decision_source == Some(LegacyDirectDecisionSource::User)
                && matches!(
                    entry.decision,
                    LegacyDirectDecisionState::Rejected | LegacyDirectDecisionState::Revoked
                )
        });
        let legacy_tombstone = self
            .legacy_direct_request_tombstones
            .iter()
            .any(|tombstone| tombstone.selector == selector);
        ignored_grant
            || ignored_contact
            || tracked_tombstone
            || user_denial
            || legacy_tombstone
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LegacyDirectRequestTombstone {
    pub selector: String,
    pub event_id: String,
    pub deleted_at: i64,
    pub retain_until: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyDirectAnswer {
    pub selector: String,
    pub decision_revision: u64,
    pub lookup_id: String,
    pub requester_device_id: String,
    pub accepted: bool,
}

pub(crate) fn legacy_selector(lookup_id: &str, peer: &DirectPeerIdentity) -> String {
    hash_id(
        "legacy-",
        b"smart-explorer/legacy-request/v1",
        [
            lookup_id,
            peer.device_id.as_str(),
            peer.public_key.as_str(),
            peer.node_id.as_str(),
        ],
    )
}

pub(super) fn evidence_from_presence(
    lookup_id: &str,
    presence: &PeerPresence,
) -> LegacyDirectPresenceEvidence {
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    LegacyDirectPresenceEvidence {
        event_id: hash_id(
            "legacy-event-",
            b"smart-explorer/legacy-presence-event/v1",
            [payload.as_str(), presence.proof.as_str()],
        ),
        relay_url: presence.relay_url.clone(),
        candidates: presence.candidates.clone(),
        expires_at: presence.expires_at,
        nonce: presence.nonce.clone(),
        proof: presence.proof.clone(),
    }
}

pub(super) fn peer_from_presence(presence: &PeerPresence) -> DirectPeerIdentity {
    DirectPeerIdentity {
        device_id: presence.device_id.clone(),
        device_name: presence.device_name.clone(),
        node_id: presence.node_id.clone(),
        public_key: presence.public_key.clone(),
        fingerprint: public_fingerprint(presence.public_key.as_bytes()),
    }
}

fn hash_id<'a>(prefix: &str, domain: &[u8], parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{prefix}{}", hex(&digest.finalize()))
}

pub(super) fn valid_hash_id(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}
