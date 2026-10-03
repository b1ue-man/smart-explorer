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
//!
//! A record denies the *key*, not only the self-chosen device id (S19): a
//! removed device that comes back under a new device id, or another install
//! that reuses its key, stays out.
use serde::{Deserialize, Serialize};

use super::direct_protocol::DirectPeerIdentity;
use super::profiles::ShareProfiles;
use super::types::{DirectContact, DirectGrant};

/// Former ledger capacity, retained for API compatibility. Denials now outlive
/// this historical threshold; forgetting one would re-authorize a removed key.
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
    /// Same device id, key, node or fingerprint. The device id is self-chosen,
    /// so the key pins count on their own; a rotated key still belongs to the
    /// device the user removed. Empty values never match.
    pub fn matches(&self, peer: &DirectPeerIdentity) -> bool {
        same(&self.device_id, &peer.device_id)
            || same(&self.public_key, &peer.public_key)
            || same(&self.node_id, &peer.node_id)
            || same(&self.fingerprint, &peer.fingerprint)
    }
}

fn same(left: &str, right: &str) -> bool {
    !left.is_empty() && left == right
}

/// Whether a reciprocal installation was requested by the user or by the
/// automatic background repair, and whether this device opens its own Direct
/// exports to the peer (FC1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingOrigin {
    /// Deliberate pairing that also opens this device's exports to the peer:
    /// the publisher of a PIN offer, or a connector that chose „Auch meine
    /// Freigaben für dieses Gerät öffnen“.
    UserPairing,
    /// Deliberate pairing that does not open this device's exports (FC1
    /// default of the connecting side): no grant is created or reactivated;
    /// an accepted grant stays as it is.
    UserPairingOneWay,
    /// Background repair: never readmits a removed peer and never reactivates
    /// an ignored or suspended („neu bestätigen“) grant.
    AutomaticRepair,
}

impl PairingOrigin {
    /// A deliberate act of the user (readmits a removed peer).
    pub fn is_user(self) -> bool {
        matches!(self, Self::UserPairing | Self::UserPairingOneWay)
    }
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

    /// Refresh the same identity, preserving every prior key/node alias.
    /// Explicit revocations never expire merely because more peers are removed.
    pub fn record_removed_direct_peer(&mut self, peer: &DirectPeerIdentity, now: i64) {
        self.removed_direct_peers
            .retain(|record| {
                record.device_id != peer.device_id
                    || record.public_key != peer.public_key
                    || record.node_id != peer.node_id
                    || record.fingerprint != peer.fingerprint
            });
        self.removed_direct_peers.push(RemovedDirectPeer {
            device_id: peer.device_id.clone(),
            device_name: peer.device_name.clone(),
            public_key: peer.public_key.clone(),
            fingerprint: peer.fingerprint.clone(),
            node_id: peer.node_id.clone(),
            removed_at: now,
        });
    }

    /// Forget the denial so the device may pair automatically again.
    pub fn readmit_removed_direct_peer(&mut self, device_id: &str) -> bool {
        let before = self.removed_direct_peers.len();
        self.removed_direct_peers
            .retain(|record| record.device_id != device_id);
        before != self.removed_direct_peers.len()
    }

    /// Lift every denial of this identity (device, key or node): a deliberate
    /// pairing readmits the peer as a whole.
    pub fn readmit_removed_direct_identity(&mut self, peer: &DirectPeerIdentity) -> bool {
        let before = self.removed_direct_peers.len();
        self.removed_direct_peers
            .retain(|record| !record.matches(peer));
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
    ///
    /// The peer's grant may predate the contact's device id (it added our code
    /// first, S29), so grants are matched by the contact's key pins as well and
    /// every matched identity is denied. A contact that never learned a device
    /// and owns no grant leaves nothing to re-install, so it records no denial.
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
        let pinned = Self::contact_remote_identity(&contact);
        let (removed_grants, kept): (Vec<DirectGrant>, Vec<DirectGrant>) =
            std::mem::take(&mut self.direct_grants)
                .into_iter()
                .partition(|grant| contact_owns_grant(&contact, pinned.as_ref(), grant));
        self.direct_grants = kept;
        let mut identities: Vec<DirectPeerIdentity> = pinned.into_iter().collect();
        for grant in &removed_grants {
            let identity = grant_identity(grant);
            if !identities
                .iter()
                .any(|known| {
                    known.device_id == identity.device_id
                        && known.public_key == identity.public_key
                        && known.node_id == identity.node_id
                })
            {
                identities.push(identity);
            }
        }
        let mut outcome = ForgottenDirectPeer {
            contact_id: contact.id.clone(),
            display_name: contact.display_name.clone(),
            identity: identities.first().cloned(),
            grants_removed: removed_grants.len(),
            ..ForgottenDirectPeer::default()
        };
        let first_device = identities
            .first()
            .map(|identity| identity.device_id.as_str());
        let (requests_removed, tombstones_skipped) =
            self.delete_direct_requests_for_forgotten_peer(contact_id, first_device, now);
        outcome.requests_removed = requests_removed;
        outcome.tombstones_skipped = tombstones_skipped;
        for identity in identities.iter().skip(1) {
            let (removed, skipped) =
                self.delete_direct_requests_for_forgotten_peer("", Some(&identity.device_id), now);
            outcome.requests_removed += removed;
            outcome.tombstones_skipped += skipped;
        }
        for identity in &identities {
            outcome.legacy_requests_removed +=
                self.delete_legacy_direct_requests_for_device(&identity.device_id, now);
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
        self.withdraw_direct_key(&identity, now);
        let _ = self.delete_direct_requests_for_forgotten_peer("", Some(device_id), now);
        let _ = self.delete_legacy_direct_requests_for_device(device_id, now);
        self.record_removed_direct_peer(&identity, now);
        self.recompute_identity_conflicts_for_device(device_id);
        true
    }
}

/// A grant belongs to the removed contact when it carries the contact's
/// device, its accepted key, or the key pins of its Direct code.
fn contact_owns_grant(
    contact: &DirectContact,
    pinned: Option<&DirectPeerIdentity>,
    grant: &DirectGrant,
) -> bool {
    let known_keys = [
        contact.remote_public_key.as_deref(),
        contact.accepted_public_key.as_deref(),
    ];
    pinned.is_some_and(|identity| identity.device_id == grant.device_id)
        || known_keys
            .into_iter()
            .flatten()
            .any(|key| same(key, &grant.public_key))
        || same(&contact.expected_node_id, &grant.node_id)
        || same(&contact.expected_fingerprint, &grant.fingerprint)
}

fn grant_identity(grant: &DirectGrant) -> DirectPeerIdentity {
    DirectPeerIdentity {
        device_id: grant.device_id.clone(),
        device_name: grant.device_name.clone(),
        node_id: grant.node_id.clone(),
        public_key: grant.public_key.clone(),
        fingerprint: grant.fingerprint.clone(),
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
            relation: Default::default(),
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
            write: false,
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
    fn review_task_removed_denials_survive_historical_capacity() {
        let mut profiles = ShareProfiles::default();
        for index in 0..(MAX_REMOVED_DIRECT_PEERS + 5) {
            profiles.record_removed_direct_peer(&identity(&format!("d{index}")), index as i64);
        }
        assert_eq!(
            profiles.removed_direct_peers.len(),
            MAX_REMOVED_DIRECT_PEERS + 5
        );
        assert!(profiles.removed_direct_peer_for_device("d0").is_some());
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
