//! Profile half of finishing a retiring Direct lookup id (E17).
use std::collections::HashSet;

use crate::share::direct_ledger::DirectRequestDirection;
use crate::share::profiles::ShareProfiles;
use crate::share::types::DirectGrantState;

use super::code_renewal::RenewalPeer;

impl ShareProfiles {
    /// After the last holder of `lookup_id` confirmed newer data: requests
    /// and request history bound to it authenticate with a secret that is
    /// gone, so they are forgotten. Grants are not touched (every holder
    /// already uses the current data). Returns the number of removed records.
    pub fn forget_retired_direct_lookup(&mut self, lookup_id: &str) -> usize {
        if lookup_id.is_empty() {
            return 0;
        }
        let mut devices = HashSet::new();
        let before = self.legacy_direct_requests.len()
            + self.direct_requests.len()
            + self.direct_request_tombstones.len();
        self.legacy_direct_requests.retain(|entry| {
            let keep = entry.lookup_id != lookup_id;
            if !keep {
                devices.insert(entry.peer.device_id.clone());
            }
            keep
        });
        self.direct_requests.retain(|entry| {
            let keep = entry.direction != DirectRequestDirection::Incoming
                || entry.local_lookup_id.as_deref() != Some(lookup_id);
            if !keep {
                devices.insert(entry.record.request.requester.device_id.clone());
            }
            keep
        });
        self.direct_request_tombstones.retain(|tombstone| {
            tombstone.direction != DirectRequestDirection::Incoming
                || tombstone.request.lookup_id != lookup_id
        });
        let after = self.legacy_direct_requests.len()
            + self.direct_requests.len()
            + self.direct_request_tombstones.len();
        for device_id in devices {
            self.recompute_identity_conflicts_for_device(&device_id);
        }
        before - after
    }

    /// Whether `lookup_id` still names any request or request history.
    pub fn names_direct_lookup(&self, lookup_id: &str) -> bool {
        self.legacy_direct_requests
            .iter()
            .any(|entry| entry.lookup_id == lookup_id)
            || self.direct_requests.iter().any(|entry| {
                entry.direction == DirectRequestDirection::Incoming
                    && entry.local_lookup_id.as_deref() == Some(lookup_id)
            })
            || self.direct_request_tombstones.iter().any(|tombstone| {
                tombstone.direction == DirectRequestDirection::Incoming
                    && tombstone.request.lookup_id == lookup_id
            })
    }

    /// Every grant that may come back with this device's Direct code: active
    /// or suspended („neu bestätigen“), never an explicit block. A removed
    /// device has no grant any more (`forget_direct_peer`).
    pub fn direct_code_holders(&self) -> Vec<RenewalPeer> {
        self.direct_grants
            .iter()
            .filter(|grant| grant.state != DirectGrantState::Ignored)
            .filter(|grant| !grant.device_id.is_empty() && !grant.public_key.is_empty())
            .map(|grant| RenewalPeer {
                device_id: grant.device_id.clone(),
                device_name: grant.device_name.clone(),
                public_key: grant.public_key.clone(),
                node_id: grant.node_id.clone(),
                legacy: false,
            })
            .collect()
    }

    /// Whether `peer` still holds a grant that may come back.
    pub fn holds_direct_code(&self, peer: &RenewalPeer) -> bool {
        self.direct_grants.iter().any(|grant| {
            grant.state != DirectGrantState::Ignored
                && peer.is(&grant.device_id, &grant.public_key, &grant.node_id)
        })
    }

    /// The latest removal of a Direct peer (contact or grant).
    pub fn latest_direct_removal(&self) -> Option<i64> {
        self.removed_direct_peers
            .iter()
            .map(|record| record.removed_at)
            .max()
    }

    /// Node ids and keys of removed devices that hold no grant again.
    pub fn withdrawn_direct_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for record in &self.removed_direct_peers {
            let readmitted = self.direct_grants.iter().any(|grant| {
                grant.state != DirectGrantState::Ignored
                    && (grant.public_key == record.public_key || grant.node_id == record.node_id)
            });
            if readmitted {
                continue;
            }
            for key in [&record.node_id, &record.public_key] {
                if !key.is_empty() && !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
        keys
    }
}
