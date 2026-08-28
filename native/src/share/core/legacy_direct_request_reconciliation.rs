use super::direct_ledger::DirectRequestDirection;
use super::direct_lifecycle::DirectDecisionState;
use super::direct_protocol::DirectPeerIdentity;
use super::direct_protocol::DirectRequestId;
use super::identity::IdentityRepairAction;
use super::legacy_direct_request::{
    LegacyDirectDecisionDelivery, LegacyDirectDecisionSource, LegacyDirectDecisionState,
    LegacyDirectDeliveryState,
};
use super::legacy_direct_request_validation::exact_grant;
use super::profiles::ShareProfiles;
use super::types::DirectGrantState;

impl ShareProfiles {
    pub(crate) fn reconcile_legacy_identity(
        &mut self,
        current_lookup_id: &str,
        now: i64,
    ) -> Result<usize, String> {
        self.prune_legacy_tombstones(now);
        let stale = self
            .legacy_direct_requests
            .iter()
            .filter(|entry| entry.lookup_id != current_lookup_id)
            .cloned()
            .collect::<Vec<_>>();
        for entry in &stale {
            if let Some(grant) = self
                .direct_grants
                .iter_mut()
                .find(|grant| exact_grant(grant, &entry.peer))
            {
                grant.state = DirectGrantState::Ignored;
                grant.updated_at = now;
                grant.exec.disable_without_decision(now);
            }
        }
        // A selector and its proof are bound to the old lookup ID and secret.
        // The identity transaction lock prevents an old event from committing
        // after rotation, so stale entries need no capacity-consuming tombstone.
        self.legacy_direct_requests
            .retain(|entry| entry.lookup_id == current_lookup_id);
        Ok(stale.len())
    }

    pub(crate) fn reconcile_legacy_grants(&mut self, now: i64) -> usize {
        let mut changed = 0;
        for entry in &mut self.legacy_direct_requests {
            let active = self.direct_grants.iter().any(|grant| {
                exact_grant(grant, &entry.peer) && grant.state == DirectGrantState::Accepted
            });
            if entry.decision == LegacyDirectDecisionState::Accepted && !active {
                entry.decision = LegacyDirectDecisionState::Revoked;
                entry.decision_source = Some(LegacyDirectDecisionSource::AuthorizationLost);
                entry.decision_changed_at = now.max(entry.decision_changed_at);
                entry.decision_revision = entry.decision_revision.saturating_add(1).max(1);
                entry.decision_delivery = LegacyDirectDecisionDelivery {
                    state: LegacyDirectDeliveryState::LocalOnlyUntracked,
                    decision_revision: entry.decision_revision,
                    ..Default::default()
                };
                changed += 1;
            }
        }
        let device_ids = self
            .legacy_direct_requests
            .iter()
            .map(|entry| entry.peer.device_id.clone())
            .collect::<std::collections::HashSet<_>>();
        for device_id in device_ids {
            self.recompute_identity_conflicts_for_device(&device_id);
        }
        changed
    }

    pub(crate) fn invalidate_direct_grants_after_identity_rotation(
        &mut self,
        current_lookup_id: &str,
        now: i64,
        action: IdentityRepairAction,
    ) -> Result<usize, String> {
        let identity_replaced = action == IdentityRepairAction::IdentityReplaced;
        let mut changed = self
            .reconcile_tracked_requests_after_identity_change(current_lookup_id, identity_replaced)
            .map_err(|error| format!("stale tracked request cleanup: {error}"))?;
        if identity_replaced {
            changed += self.legacy_direct_requests.len();
            self.legacy_direct_requests.clear();
        } else {
            changed += self.reconcile_legacy_identity(current_lookup_id, now)?;
        }
        // These tombstones authenticate only events from the obsolete direct
        // secret. Clearing them is safe after rotation and makes recovery
        // independent of a saturated deletion ledger.
        changed += self.legacy_direct_request_tombstones.len();
        self.legacy_direct_request_tombstones.clear();
        if identity_replaced {
            changed += self.reset_outgoing_authorization_after_identity_replacement();
        }
        changed += self.invalidate_all_direct_grants(now);
        Ok(changed)
    }

    pub(crate) fn invalidate_all_direct_grants(&mut self, now: i64) -> usize {
        let mut changed = 0;
        for grant in &mut self.direct_grants {
            if grant.state == DirectGrantState::Accepted || grant.exec.enabled {
                grant.state = DirectGrantState::Ignored;
                grant.updated_at = now;
                grant.exec.disable_without_decision(now);
                changed += 1;
            }
        }
        changed += self.reconcile_legacy_grants(now);
        changed
    }

    pub(crate) fn mark_legacy_revoked_for_peer(
        &mut self,
        peer: &DirectPeerIdentity,
        now: i64,
    ) -> usize {
        let mut changed = 0;
        for entry in &mut self.legacy_direct_requests {
            if entry.peer.device_id == peer.device_id
                && entry.peer.public_key == peer.public_key
                && entry.peer.node_id == peer.node_id
                && entry.decision == LegacyDirectDecisionState::Accepted
            {
                mark_revoked(entry, LegacyDirectDecisionSource::User, now);
                changed += 1;
            }
        }
        changed
    }

    pub(super) fn identity_conflicts(&self, peer: &DirectPeerIdentity, selector: &str) -> bool {
        self.direct_grants.iter().any(|grant| {
            grant.state == DirectGrantState::Accepted
                && grant.device_id == peer.device_id
                && !exact_grant(grant, peer)
        }) || self.legacy_direct_requests.iter().any(|entry| {
            entry.selector != selector
                && live_legacy_identity_claim(entry)
                && entry.peer.device_id == peer.device_id
                && (entry.peer.public_key != peer.public_key || entry.peer.node_id != peer.node_id)
        }) || self.direct_requests.iter().any(|entry| {
            entry.direction == DirectRequestDirection::Incoming
                && live_tracked_identity_claim(entry)
                && peer_identity_conflicts(&entry.record.request.requester, peer)
        })
    }

    pub(crate) fn tracked_identity_conflict(&self, request_id: &DirectRequestId) -> bool {
        let Some(entry) = self.direct_request(request_id) else {
            return false;
        };
        if entry.direction != DirectRequestDirection::Incoming {
            return false;
        }
        if !live_tracked_identity_claim(entry) {
            return false;
        }
        let peer = &entry.record.request.requester;
        self.direct_grants.iter().any(|grant| {
            grant.state == DirectGrantState::Accepted
                && grant.device_id == peer.device_id
                && !exact_grant(grant, peer)
        }) || self.direct_requests.iter().any(|sibling| {
            sibling.direction == DirectRequestDirection::Incoming
                && sibling.record.request.request_id != *request_id
                && live_tracked_identity_claim(sibling)
                && peer_identity_conflicts(&sibling.record.request.requester, peer)
        }) || self.legacy_direct_requests.iter().any(|sibling| {
            live_legacy_identity_claim(sibling) && peer_identity_conflicts(&sibling.peer, peer)
        })
    }

    pub(super) fn recompute_identity_conflicts_for_device(&mut self, device_id: &str) {
        let pins = self
            .legacy_direct_requests
            .iter()
            .filter(|entry| entry.peer.device_id == device_id && live_legacy_identity_claim(entry))
            .map(|entry| {
                (
                    entry.peer.public_key.clone(),
                    entry.peer.node_id.clone(),
                    entry.peer.fingerprint.clone(),
                )
            })
            .chain(
                self.direct_requests
                    .iter()
                    .filter(|entry| {
                        entry.direction == DirectRequestDirection::Incoming
                            && live_tracked_identity_claim(entry)
                            && entry.record.request.requester.device_id == device_id
                    })
                    .map(|entry| {
                        let peer = &entry.record.request.requester;
                        (
                            peer.public_key.clone(),
                            peer.node_id.clone(),
                            peer.fingerprint.clone(),
                        )
                    }),
            )
            .collect::<Vec<_>>();
        let grant = self
            .direct_grants
            .iter()
            .find(|grant| grant.device_id == device_id && grant.state == DirectGrantState::Accepted)
            .map(|grant| {
                (
                    grant.public_key.as_str(),
                    grant.node_id.as_str(),
                    grant.fingerprint.as_str(),
                )
            });
        for entry in self
            .legacy_direct_requests
            .iter_mut()
            .filter(|entry| entry.peer.device_id == device_id)
        {
            // Keep the mismatch visible on the rejected request that records it;
            // rejection removes a live claim, not the conflict evidence itself.
            entry.identity_conflict = pins.iter().any(|(public_key, node_id, fingerprint)| {
                *public_key != entry.peer.public_key
                    || *node_id != entry.peer.node_id
                    || *fingerprint != entry.peer.fingerprint
            }) || grant.is_some_and(|(public_key, node_id, fingerprint)| {
                public_key != entry.peer.public_key
                    || node_id != entry.peer.node_id
                    || fingerprint != entry.peer.fingerprint
            });
        }
    }

    pub(super) fn recompute_all_identity_conflicts(&mut self) {
        let device_ids = self
            .legacy_direct_requests
            .iter()
            .map(|entry| entry.peer.device_id.clone())
            .collect::<std::collections::HashSet<_>>();
        for device_id in device_ids {
            self.recompute_identity_conflicts_for_device(&device_id);
        }
    }

    pub(super) fn prune_legacy_tombstones(&mut self, now: i64) {
        self.legacy_direct_request_tombstones
            .retain(|entry| entry.retain_until >= now);
    }
}

fn peer_identity_conflicts(left: &DirectPeerIdentity, right: &DirectPeerIdentity) -> bool {
    left.device_id == right.device_id
        && (left.public_key != right.public_key
            || left.node_id != right.node_id
            || left.fingerprint != right.fingerprint)
}

fn live_tracked_identity_claim(entry: &super::direct_ledger::DirectRequestEntry) -> bool {
    matches!(
        entry.record.decision.state,
        DirectDecisionState::Pending | DirectDecisionState::Accepted
    )
}

fn live_legacy_identity_claim(
    entry: &super::legacy_direct_request::LegacyDirectRequestEntry,
) -> bool {
    matches!(
        entry.decision,
        LegacyDirectDecisionState::Pending | LegacyDirectDecisionState::Accepted
    )
}

fn mark_revoked(
    entry: &mut super::legacy_direct_request::LegacyDirectRequestEntry,
    source: LegacyDirectDecisionSource,
    now: i64,
) {
    entry.decision = LegacyDirectDecisionState::Revoked;
    entry.decision_source = Some(source);
    entry.decision_changed_at = now.max(entry.decision_changed_at);
    entry.decision_revision = entry.decision_revision.saturating_add(1).max(1);
    entry.decision_delivery = LegacyDirectDecisionDelivery {
        state: LegacyDirectDeliveryState::LocalOnlyUntracked,
        decision_revision: entry.decision_revision,
        ..Default::default()
    };
}
