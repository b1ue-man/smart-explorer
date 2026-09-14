//! Durable record of Direct peers the user removed.
//!
//! Removing a Direct contact must delete everything the relation created:
//! the contact, the peer's grant (including its Exec grant), every tracked
//! and legacy request of that device, and the relation secret. The peer keeps
//! our Direct code and its own half of the relation, so without a durable
//! denial it would re-install itself within seconds through the automatic
//! reciprocal repair or an auto-accepted access request. A `RemovedDirectPeer`
//! blocks exactly those *automatic* paths. Any deliberate pairing by the user
//! (PIN discovery, adding a Direct code, accepting an incoming request) clears
//! the record again.
use serde::{Deserialize, Serialize};

use super::direct_protocol::DirectPeerIdentity;
use super::profiles::ShareProfiles;
use super::types::DirectContact;

/// Records are small, but a bounded ledger keeps the profile file bounded.
pub const MAX_REMOVED_DIRECT_PEERS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemovedDirectPeer {
    pub device_id: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub node_id: String,
    pub removed_at: i64,
}

impl RemovedDirectPeer {
    /// The device id is the stable installation identity; a rotated key still
    /// belongs to the device the user removed.
    pub fn matches(&self, peer: &DirectPeerIdentity) -> bool {
        self.device_id == peer.device_id
    }
}

/// Whether a reciprocal installation was requested by the user or by the
/// automatic background repair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingOrigin {
    UserPairing,
    AutomaticRepair,
}

/// Everything one removal transaction deleted, for reporting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgottenDirectPeer {
    pub contact_id: String,
    pub display_name: String,
    pub identity: Option<DirectPeerIdentity>,
    pub grants_removed: usize,
    pub requests_removed: usize,
    pub legacy_requests_removed: usize,
    pub tombstones_skipped: usize,
}

impl ShareProfiles {
    pub fn removed_direct_peer(&self, peer: &DirectPeerIdentity) -> Option<&RemovedDirectPeer> {
        self.removed_direct_peers
            .iter()
            .find(|record| record.matches(peer))
    }

    pub fn removed_direct_peer_for_device(&self, device_id: &str) -> Option<&RemovedDirectPeer> {
        self.removed_direct_peers
            .iter()
            .find(|record| record.device_id == device_id)
    }

    /// Replace any record for the same device and keep the ledger bounded by
    /// dropping the oldest records first.
    pub fn record_removed_direct_peer(&mut self, peer: &DirectPeerIdentity, now: i64) {
        self.removed_direct_peers
            .retain(|record| record.device_id != peer.device_id);
        self.removed_direct_peers.push(RemovedDirectPeer {
            device_id: peer.device_id.clone(),
            device_name: peer.device_name.clone(),
            public_key: peer.public_key.clone(),
            fingerprint: peer.fingerprint.clone(),
            node_id: peer.node_id.clone(),
            removed_at: now,
        });
        if self.removed_direct_peers.len() > MAX_REMOVED_DIRECT_PEERS {
            self.removed_direct_peers
                .sort_by_key(|record| std::cmp::Reverse(record.removed_at));
            self.removed_direct_peers.truncate(MAX_REMOVED_DIRECT_PEERS);
        }
    }

    /// Forget the denial so the device may pair automatically again.
    pub fn readmit_removed_direct_peer(&mut self, device_id: &str) -> bool {
        let before = self.removed_direct_peers.len();
        self.removed_direct_peers
            .retain(|record| record.device_id != device_id);
        before != self.removed_direct_peers.len()
    }

    /// The pinned identity of a contact's remote device, when the relation was
    /// ever accepted. A pending outgoing contact has no device to deny.
    pub fn contact_remote_identity(contact: &DirectContact) -> Option<DirectPeerIdentity> {
        let device_id = contact
            .remote_device_id
            .clone()
            .filter(|id| !id.trim().is_empty())?;
        let public_key = contact
            .remote_public_key
            .clone()
            .or_else(|| contact.accepted_public_key.clone())
            .unwrap_or_default();
        Some(DirectPeerIdentity {
            device_id,
            device_name: contact.display_name.clone(),
            node_id: contact.expected_node_id.clone(),
            public_key,
            fingerprint: contact.expected_fingerprint.clone(),
        })
    }

    /// The in-memory half of "remove this Direct peer completely". Returns
    /// `None` when no such contact exists (already forgotten). Idempotent, so a
    /// persisted compare-and-swap may replay it.
    pub fn forget_direct_peer(
        &mut self,
        contact_id: &str,
        now: i64,
    ) -> Option<ForgottenDirectPeer> {
        let index = self
            .direct_contacts
            .iter()
            .position(|contact| contact.id == contact_id)?;
        let contact = self.direct_contacts.remove(index);
        let identity = Self::contact_remote_identity(&contact);
        let mut outcome = ForgottenDirectPeer {
            contact_id: contact.id.clone(),
            display_name: contact.display_name.clone(),
            identity: identity.clone(),
            ..ForgottenDirectPeer::default()
        };
        let device_id = identity
            .as_ref()
            .map(|identity| identity.device_id.as_str());
        if let Some(device_id) = device_id {
            let before = self.direct_grants.len();
            self.direct_grants
                .retain(|grant| grant.device_id != device_id);
            outcome.grants_removed = before - self.direct_grants.len();
        }
        let (requests_removed, tombstones_skipped) =
            self.delete_direct_requests_for_forgotten_peer(contact_id, device_id, now);
        outcome.requests_removed = requests_removed;
        outcome.tombstones_skipped = tombstones_skipped;
        if let Some(device_id) = device_id {
            outcome.legacy_requests_removed =
                self.delete_legacy_direct_requests_for_device(device_id, now);
        }
        if let Some(identity) = &identity {
            self.record_removed_direct_peer(identity, now);
            self.recompute_identity_conflicts_for_device(&identity.device_id);
        }
        Some(outcome)
    }

    /// Delete a leftover grant (active or inactive) together with the requests
    /// that carry it, and deny automatic re-installation. Returns `false` when
    /// no grant for the device exists.
    pub fn delete_direct_grant(&mut self, device_id: &str, now: i64) -> bool {
        let Some(index) = self
            .direct_grants
            .iter()
            .position(|grant| grant.device_id == device_id)
        else {
            return false;
        };
        let grant = self.direct_grants.remove(index);
        self.direct_grants
            .retain(|remaining| remaining.device_id != device_id);
        let identity = DirectPeerIdentity {
            device_id: grant.device_id.clone(),
            device_name: grant.device_name.clone(),
            node_id: grant.node_id.clone(),
            public_key: grant.public_key.clone(),
            fingerprint: grant.fingerprint.clone(),
        };
        let _ = self.delete_direct_requests_for_forgotten_peer("", Some(device_id), now);
        let _ = self.delete_legacy_direct_requests_for_device(device_id, now);
        self.record_removed_direct_peer(&identity, now);
        self.recompute_identity_conflicts_for_device(device_id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::types::{DirectAccessState, DirectGrant, DirectGrantState, ShareStatus};

    fn identity(device_id: &str) -> DirectPeerIdentity {
        DirectPeerIdentity {
            device_id: device_id.into(),
            device_name: format!("Device {device_id}"),
            node_id: format!("node-{device_id}"),
            public_key: format!("key-{device_id}"),
            fingerprint: format!("fp-{device_id}"),
        }
    }

    fn accepted_contact(id: &str, device_id: &str) -> DirectContact {
        DirectContact {
            id: id.into(),
            display_name: format!("Contact {id}"),
            lookup_id: format!("lookup-{id}"),
            expected_fingerprint: format!("fp-{device_id}"),
            expected_node_id: format!("node-{device_id}"),
            remote_device_id: Some(device_id.into()),
            remote_public_key: Some(format!("key-{device_id}")),
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Offline,
            last_error: None,
            presence: None,
            access_state: DirectAccessState::Accepted,
            request_sent_at: None,
            accepted_at: Some(1),
            accepted_public_key: Some(format!("key-{device_id}")),
            lan_candidates: Vec::new(),
            lan_seen_at: None,
            lan_uplink: None,
        }
    }

    fn grant(device_id: &str) -> DirectGrant {
        DirectGrant {
            device_id: device_id.into(),
            device_name: format!("Device {device_id}"),
            public_key: format!("key-{device_id}"),
            fingerprint: format!("fp-{device_id}"),
            node_id: format!("node-{device_id}"),
            state: DirectGrantState::Accepted,
            updated_at: 1,
            exec: Default::default(),
        }
    }

    #[test]
    fn lan_cleanup_task_record_lookup_and_readmit() {
        let mut profiles = ShareProfiles::default();
        profiles.record_removed_direct_peer(&identity("a"), 10);
        assert!(profiles.removed_direct_peer(&identity("a")).is_some());
        assert!(profiles.removed_direct_peer_for_device("a").is_some());
        assert!(profiles.removed_direct_peer(&identity("b")).is_none());
        let mut rotated = identity("a");
        rotated.public_key = "other".into();
        assert!(profiles.removed_direct_peer(&rotated).is_some());
        assert!(profiles.readmit_removed_direct_peer("a"));
        assert!(!profiles.readmit_removed_direct_peer("a"));
        assert!(profiles.removed_direct_peers.is_empty());
    }

    #[test]
    fn lan_cleanup_task_ledger_is_bounded_and_keeps_newest_records() {
        let mut profiles = ShareProfiles::default();
        for index in 0..(MAX_REMOVED_DIRECT_PEERS + 5) {
            profiles.record_removed_direct_peer(&identity(&format!("d{index}")), index as i64);
        }
        assert_eq!(
            profiles.removed_direct_peers.len(),
            MAX_REMOVED_DIRECT_PEERS
        );
        assert!(profiles.removed_direct_peer_for_device("d0").is_none());
        assert!(profiles
            .removed_direct_peer_for_device(&format!("d{}", MAX_REMOVED_DIRECT_PEERS + 4))
            .is_some());
    }

    #[test]
    fn lan_cleanup_task_re_recording_a_device_replaces_its_record() {
        let mut profiles = ShareProfiles::default();
        profiles.record_removed_direct_peer(&identity("a"), 1);
        profiles.record_removed_direct_peer(&identity("a"), 2);
        assert_eq!(profiles.removed_direct_peers.len(), 1);
        assert_eq!(profiles.removed_direct_peers[0].removed_at, 2);
    }

    #[test]
    fn lan_cleanup_task_forgetting_removes_contact_grant_and_records_denial() {
        let mut profiles = ShareProfiles::default();
        profiles.direct_contacts.push(accepted_contact("c1", "a"));
        profiles.direct_contacts.push(accepted_contact("c2", "b"));
        profiles.direct_grants.push(grant("a"));
        profiles.direct_grants.push(grant("b"));

        let outcome = profiles
            .forget_direct_peer("c1", 50)
            .expect("contact exists");
        assert_eq!(outcome.contact_id, "c1");
        assert_eq!(outcome.grants_removed, 1);
        assert_eq!(
            outcome.identity.as_ref().map(|i| i.device_id.as_str()),
            Some("a")
        );
        assert!(profiles.direct_contacts.iter().all(|c| c.id != "c1"));
        assert!(profiles.direct_grants.iter().all(|g| g.device_id != "a"));
        assert_eq!(profiles.direct_grants.len(), 1);
        assert!(profiles.removed_direct_peer_for_device("a").is_some());
        assert!(profiles.removed_direct_peer_for_device("b").is_none());
        assert!(profiles.forget_direct_peer("c1", 51).is_none());
    }

    #[test]
    fn lan_cleanup_task_forgetting_a_pending_contact_records_no_denial() {
        let mut profiles = ShareProfiles::default();
        let mut pending = accepted_contact("c1", "a");
        pending.remote_device_id = None;
        pending.access_state = DirectAccessState::Pending;
        profiles.direct_contacts.push(pending);
        let outcome = profiles
            .forget_direct_peer("c1", 5)
            .expect("contact exists");
        assert!(outcome.identity.is_none());
        assert!(profiles.removed_direct_peers.is_empty());
        assert!(profiles.direct_contacts.is_empty());
    }

    #[test]
    fn lan_cleanup_task_deleting_a_grant_records_denial_for_its_device() {
        let mut profiles = ShareProfiles::default();
        let mut inactive = grant("a");
        inactive.state = DirectGrantState::Ignored;
        profiles.direct_grants.push(inactive);
        assert!(profiles.delete_direct_grant("a", 7));
        assert!(profiles.direct_grants.is_empty());
        assert_eq!(
            profiles
                .removed_direct_peer_for_device("a")
                .map(|record| record.fingerprint.as_str()),
            Some("fp-a")
        );
        assert!(!profiles.delete_direct_grant("a", 8));
    }
}
