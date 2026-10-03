//! Explicit, read-only admission proof for an unconfirmed legacy Direct peer.
//! A signaling answer is never decision evidence. Only the pinned peer's
//! successful filesystem probe may confirm the existing outgoing contact.
use std::sync::atomic::Ordering;

use super::backend::PeerBackend;
use super::core::{now_secs, presence_payload, verify_hmac};
use super::direct_ledger::{DirectRelayOutcome, DirectRequestDirection, DirectRequestEntry};
use super::direct_lifecycle::DirectDecisionState;
use super::direct_protocol::DirectPeerIdentity;
use super::identity::ShareIdentity;
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::service::ShareService;
use super::signal_presence::PresenceSignature;
use super::types::{
    DirectAccessState, DirectContact, PeerEndpoint, PeerOpenTarget, ShareAuthState, ShareScope,
};

const CHANGED: &str = "Legacy-Freigabe hat sich geaendert; Anfrage neu laden";

/// Secret-bearing binding for one user-initiated probe; never logged or sent.
pub(super) struct PendingLegacyProbe {
    pub(super) identity: ShareIdentity,
    pub(super) endpoint: PeerEndpoint,
    contact: DirectContact,
    request: DirectRequestEntry,
}

pub(super) fn confirm_pending(
    service: &ShareService,
    target: &PeerOpenTarget,
    endpoint: &PeerEndpoint,
) -> Result<bool, String> {
    check_running(service)?;
    let Some(probe) = PendingLegacyProbe::capture(service, target, endpoint)? else {
        return Ok(false);
    };
    super::legacy_probe_persist::validate_before_probe(service, &probe)?;
    // No configuration permit, identity lock or auth lock crosses peer I/O.
    let backend = PeerBackend::new(
        probe.endpoint.clone(),
        probe.identity.clone(),
        service.iroh.clone(),
    );
    backend.probe_legacy_root().map_err(|error| error.to_string())?;
    super::legacy_probe_persist::persist(service, &probe)?;
    Ok(true)
}

pub(super) fn check_running(service: &ShareService) -> Result<(), String> {
    if service.stopped.load(Ordering::Acquire) {
        return Err("Share ist gestoppt".into());
    }
    service
        .iroh
        .require_sharing_active()
        .map_err(|error| error.to_string())
}

impl PendingLegacyProbe {
    fn capture(
        service: &ShareService,
        target: &PeerOpenTarget,
        endpoint: &PeerEndpoint,
    ) -> Result<Option<Self>, String> {
        let PeerOpenTarget::Direct { contact_id } = target else {
            return Ok(None);
        };
        let state = service.auth.lock().map_err(|_| "Share-State gesperrt")?;
        let contact = state
            .direct_contacts
            .iter()
            .find(|contact| &contact.id == contact_id)
            .ok_or_else(|| CHANGED.to_string())?;
        if contact.access_state != DirectAccessState::Pending {
            return Ok(None);
        }
        let request = bound_request(&state.direct_requests, contact_id)?;
        // Modern pending requests continue through the ordinary denied path.
        if request.retries.request.relay_outcome != Some(DirectRelayOutcome::LegacyForwarded) {
            return Ok(None);
        }
        let probe = Self {
            identity: service.identity.clone(),
            endpoint: endpoint.clone(),
            contact: contact.clone(),
            request: request.clone(),
        };
        probe.check_auth(&state)?;
        Ok(Some(probe))
    }

    pub(super) fn check_auth(&self, state: &ShareAuthState) -> Result<(), String> {
        if !state.direct_online
            || !same_identity(&self.identity, &state.identity)
            || state.direct_secret != self.identity.direct_secret()
            || super::signal_auth::peer_signature_seen(state, &self.endpoint.presence)
        {
            return Err(CHANGED.into());
        }
        let runtime_policy = ShareProfiles {
            direct_contacts: state.direct_contacts.clone(),
            direct_grants: state.direct_grants.clone(),
            direct_request_tombstones: state.direct_request_tombstones.clone(),
            ..ShareProfiles::default()
        };
        if runtime_policy.direct_auto_accept_denied(&self.identity.direct_lookup_id, &self.peer()) {
            return Err("Direktgeraet wurde gesperrt oder entfernt".into());
        }
        let contact = state
            .direct_contacts
            .iter()
            .find(|contact| contact.id == self.contact.id)
            .ok_or_else(|| CHANGED.to_string())?;
        self.check_contact(contact)?;
        self.check_request(bound_request(&state.direct_requests, &contact.id)?)
    }

    pub(super) fn check_profiles(&self, profiles: &ShareProfiles) -> Result<(), String> {
        let contact = profiles
            .direct_contacts
            .iter()
            .find(|contact| contact.id == self.contact.id)
            .ok_or_else(|| CHANGED.to_string())?;
        self.check_contact(contact)?;
        self.check_request(bound_request(&profiles.direct_requests, &contact.id)?)?;
        if profiles.direct_auto_accept_denied(&self.identity.direct_lookup_id, &self.peer()) {
            return Err("Direktgeraet wurde gesperrt oder entfernt".into());
        }
        Ok(())
    }

    pub(super) fn contact_id(&self) -> &str {
        &self.contact.id
    }

    fn check_contact(&self, contact: &DirectContact) -> Result<(), String> {
        let peer = &self.endpoint.presence;
        let ShareScope::Direct { contact_id } = &self.endpoint.scope else {
            return Err(CHANGED.into());
        };
        if contact.id != *contact_id
            || contact.lookup_id != self.contact.lookup_id
            || contact.expected_fingerprint != self.contact.expected_fingerprint
            || contact.expected_node_id != self.contact.expected_node_id
            || contact.remote_device_id != self.contact.remote_device_id
            || contact.remote_public_key != self.contact.remote_public_key
            || contact.access_state != DirectAccessState::Pending
            || contact.accepted_at.is_some()
            || contact.accepted_public_key.is_some()
            || contact.relation.signed_presence
            || contact.expected_node_id.is_empty()
            || contact.expected_node_id != peer.node_id
            || self.endpoint.expected_node_id.as_deref() != Some(contact.expected_node_id.as_str())
            || contact.remote_device_id.as_deref() != Some(peer.device_id.as_str())
            || contact.remote_public_key.as_deref() != Some(peer.public_key.as_str())
            || !fingerprint_matches(&peer.public_key, &contact.expected_fingerprint)
            || peer.signature_state() != PresenceSignature::Unsigned
            || !peer.is_current_at(now_secs())
            || peer.kind != "direct"
            || peer.relation_id != contact.lookup_id
        {
            return Err(CHANGED.into());
        }
        self.peer().validate().map_err(|error| error.to_string())?;
        let secret = ShareProfiles::direct_secret_checked(contact)?
            .ok_or_else(|| "Direkt-Secret fehlt".to_string())?;
        if secret != self.endpoint.relation_secret {
            return Err(CHANGED.into());
        }
        let payload = presence_payload(
            "direct",
            &peer.relation_id,
            &peer.device_id,
            &peer.public_key,
            &peer.node_id,
            &peer.relay_url,
            &peer.candidates,
            peer.expires_at,
            &peer.nonce,
        );
        if !verify_hmac(&secret, &payload, &peer.proof) {
            return Err(CHANGED.into());
        }
        Ok(())
    }

    fn check_request(&self, entry: &DirectRequestEntry) -> Result<(), String> {
        let request = &entry.record.request;
        let peer = &self.endpoint.presence;
        if entry != &self.request
            || entry.direction != DirectRequestDirection::Outgoing
            || entry
                .local_lookup_id
                .as_deref()
                .is_some_and(|lookup| lookup != self.identity.direct_lookup_id.as_str())
            || entry.request_receipt.is_some()
            || entry.decision.is_some()
            || entry.decision_receipt.is_some()
            || entry.record.decision.state != DirectDecisionState::Pending
            || entry.record.decision.revision != 0
            || entry.retries.request.relay_outcome != Some(DirectRelayOutcome::LegacyForwarded)
            || request.lookup_id != self.contact.lookup_id
            || request.requester.device_id != self.identity.device_id
            || request.requester.public_key != self.identity.public_key
            || request.requester.node_id != self.identity.node_id
            || request.requester.fingerprint != self.identity.fingerprint
            || request.target.public_key != peer.public_key
            || request.target.node_id != peer.node_id
            || request.target.fingerprint != peer.fingerprint
            || (!request.target.device_id.is_empty() && request.target.device_id != peer.device_id)
        {
            return Err(CHANGED.into());
        }
        request
            .verify_at(&self.endpoint.relation_secret, now_secs())
            .map_err(|error| error.to_string())
    }

    fn peer(&self) -> DirectPeerIdentity {
        let peer = &self.endpoint.presence;
        DirectPeerIdentity {
            device_id: peer.device_id.clone(),
            device_name: String::new(),
            public_key: peer.public_key.clone(),
            node_id: peer.node_id.clone(),
            fingerprint: peer.fingerprint.clone(),
        }
    }
}

fn bound_request<'a>(
    entries: &'a [DirectRequestEntry],
    contact_id: &str,
) -> Result<&'a DirectRequestEntry, String> {
    let mut matches = entries.iter().filter(|entry| {
        entry.direction == DirectRequestDirection::Outgoing
            && entry.contact_id.as_deref() == Some(contact_id)
    });
    let entry = matches.next().ok_or_else(|| CHANGED.to_string())?;
    if matches.next().is_some() {
        return Err(CHANGED.into());
    }
    Ok(entry)
}

fn same_identity(left: &ShareIdentity, right: &ShareIdentity) -> bool {
    left.device_id == right.device_id
        && left.public_key == right.public_key
        && left.node_id == right.node_id
        && left.fingerprint == right.fingerprint
        && left.direct_lookup_id == right.direct_lookup_id
        && left.direct_secret == right.direct_secret
}
