use super::direct_ledger::{
    DirectLedgerError, DirectRequestDirection, DirectRequestEntry, MAX_DIRECT_REQUEST_ENTRIES,
};
use super::direct_lifecycle::DirectDecisionState;
use super::direct_protocol::{validate_persisted_timestamp, MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS};
use super::direct_request_tombstone::{tombstone_for, MAX_DIRECT_REQUEST_TOMBSTONES};
use super::legacy_direct_request_validation::exact_grant;
use super::profiles::ShareProfiles;
use super::types::DirectAccessState;

impl ShareProfiles {
    /// Prunes completed request history when the ledger is at capacity.
    ///
    /// Eviction order (oldest `retention_timestamp` first within each tier):
    /// 1. Inactive terminal history (`Rejected`/`Revoked` after peer receipt or decision expiry,
    ///    `Failed`, `Expired`).
    /// 2. Settled incoming `Accepted` entries whose delivery is finished and whose authority lives
    ///    in `direct_grants` (or `removed_direct_peers`). No incoming tombstone is written because
    ///    incoming tombstones represent user dismissals/denials (`direct_auto_accept_denied`).
    /// 3. Settled outgoing `Accepted` entries whose `DecisionReceipt` is finished and whose
    ///    authority lives in `direct_contacts`, retaining a best-effort outgoing `HistoryDeleted`
    ///    tombstone so a later signed remote revocation still applies.
    ///
    /// `Pending` requests and any entry with open or manually retryable envelopes are never evicted.
    pub fn prune_direct_request_history(&mut self, now: i64) -> usize {
        if now < 0 {
            return 0;
        }
        let before = self.direct_requests.len();
        while self.direct_requests.len() >= MAX_DIRECT_REQUEST_ENTRIES {
            let index = {
                let profiles: &Self = self;
                oldest_matching(&profiles.direct_requests, |entry| {
                    can_prune_inactive_terminal(entry, now)
                })
                .or_else(|| {
                    oldest_matching(&profiles.direct_requests, |entry| {
                        can_prune_settled_incoming_accepted(profiles, entry, now)
                    })
                })
                .or_else(|| {
                    oldest_matching(&profiles.direct_requests, |entry| {
                        can_prune_settled_outgoing_accepted(profiles, entry, now)
                    })
                })
            };
            let Some(index) = index else {
                break;
            };
            let removed = self.direct_requests.remove(index);
            match removed.direction {
                DirectRequestDirection::Incoming => {
                    self.recompute_identity_conflicts_for_device(
                        &removed.record.request.requester.device_id,
                    );
                }
                DirectRequestDirection::Outgoing
                    if removed.record.decision.state == DirectDecisionState::Accepted =>
                {
                    self.retain_pruned_outgoing_accept_tombstone(&removed, now);
                }
                DirectRequestDirection::Outgoing => {}
            }
        }
        before - self.direct_requests.len()
    }

    pub(super) fn ensure_direct_request_capacity(
        &mut self,
        now: i64,
    ) -> Result<(), DirectLedgerError> {
        if self.direct_requests.len() >= MAX_DIRECT_REQUEST_ENTRIES {
            self.prune_direct_request_history(now);
        }
        if self.direct_requests.len() < MAX_DIRECT_REQUEST_ENTRIES {
            Ok(())
        } else {
            Err(DirectLedgerError::LedgerFull)
        }
    }

    fn retain_pruned_outgoing_accept_tombstone(&mut self, entry: &DirectRequestEntry, now: i64) {
        self.prune_direct_request_tombstones(now);
        if self.direct_request_tombstones.len() >= MAX_DIRECT_REQUEST_TOMBSTONES
            || self
                .direct_request_tombstone(&entry.record.request.request_id)
                .is_some()
            || entry.record.decision.revision == 0
            || entry.contact_id.as_deref().is_none_or(str::is_empty)
            || validate_persisted_timestamp(
                entry.record.request.created_at,
                entry.record.request.expires_at,
            )
            .is_err()
            || entry.record.request.digest().is_err()
        {
            return;
        }
        if let Ok(tombstone) = tombstone_for(entry, now) {
            self.direct_request_tombstones.push(tombstone);
        }
    }
}

fn oldest_matching(
    entries: &[DirectRequestEntry],
    predicate: impl Fn(&DirectRequestEntry) -> bool,
) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| predicate(entry))
        .min_by_key(|(_, entry)| retention_timestamp(entry))
        .map(|(index, _)| index)
}

fn has_open_envelopes(entry: &DirectRequestEntry, now: i64) -> bool {
    !entry.pending_outboxes(now).is_empty() || !entry.manually_retryable_outboxes(now).is_empty()
}

fn within_retention_clock(entry: &DirectRequestEntry, now: i64, allow_peer_skew: bool) -> bool {
    let limit = if allow_peer_skew {
        now.saturating_add(MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS)
    } else {
        now
    };
    retention_timestamp(entry) <= limit
}

fn decision_settled(entry: &DirectRequestEntry, now: i64) -> bool {
    entry.decision_receipt.is_some()
        || entry
            .decision
            .as_ref()
            .is_some_and(|decision| decision.expires_at < now)
}

fn can_prune_inactive_terminal(entry: &DirectRequestEntry, now: i64) -> bool {
    if has_open_envelopes(entry, now) {
        return false;
    }
    match entry.record.decision.state {
        DirectDecisionState::Rejected | DirectDecisionState::Revoked => {
            decision_settled(entry, now) && within_retention_clock(entry, now, true)
        }
        DirectDecisionState::Failed | DirectDecisionState::Expired => {
            within_retention_clock(entry, now, false)
        }
        DirectDecisionState::Pending | DirectDecisionState::Accepted => false,
    }
}

fn can_prune_settled_incoming_accepted(
    profiles: &ShareProfiles,
    entry: &DirectRequestEntry,
    now: i64,
) -> bool {
    entry.direction == DirectRequestDirection::Incoming
        && entry.record.decision.state == DirectDecisionState::Accepted
        && entry.decision.is_some()
        && decision_settled(entry, now)
        && !has_open_envelopes(entry, now)
        && within_retention_clock(entry, now, true)
        && settled_in_grant(profiles, entry)
}

fn can_prune_settled_outgoing_accepted(
    profiles: &ShareProfiles,
    entry: &DirectRequestEntry,
    now: i64,
) -> bool {
    entry.direction == DirectRequestDirection::Outgoing
        && entry.record.decision.state == DirectDecisionState::Accepted
        && entry.decision.is_some()
        && entry.decision_receipt.is_some()
        && !has_open_envelopes(entry, now)
        && within_retention_clock(entry, now, true)
        && settled_in_contact(profiles, entry)
}

fn settled_in_grant(profiles: &ShareProfiles, entry: &DirectRequestEntry) -> bool {
    let peer = &entry.record.request.requester;
    profiles
        .direct_grants
        .iter()
        .any(|grant| exact_grant(grant, peer))
        || profiles.removed_direct_peer(peer).is_some()
}

fn settled_in_contact(profiles: &ShareProfiles, entry: &DirectRequestEntry) -> bool {
    let Some(contact_id) = entry.contact_id.as_deref().filter(|id| !id.is_empty()) else {
        return false;
    };
    profiles.direct_contacts.iter().any(|contact| {
        contact.id == contact_id
            && (contact.access_state != DirectAccessState::Pending
                || contact.accepted_public_key.is_some())
    })
}

fn retention_timestamp(entry: &DirectRequestEntry) -> i64 {
    entry
        .record
        .delivery
        .changed_at
        .max(entry.record.decision.changed_at)
        .max(entry.record.decision_delivery.changed_at)
}
