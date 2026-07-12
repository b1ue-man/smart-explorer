use super::direct_ledger::{DirectLedgerError, DirectRequestEntry, MAX_DIRECT_REQUEST_ENTRIES};
use super::direct_lifecycle::DirectDecisionState;
use super::profiles::ShareProfiles;

impl ShareProfiles {
    /// Prunes only inactive terminal history when the ledger is at capacity.
    /// Pending and accepted requests, plus any retained envelope that remains
    /// retryable, are never evicted.
    pub fn prune_direct_request_history(&mut self, now: i64) -> usize {
        let before = self.direct_requests.len();
        while self.direct_requests.len() >= MAX_DIRECT_REQUEST_ENTRIES {
            let Some(index) = self
                .direct_requests
                .iter()
                .enumerate()
                .filter(|(_, entry)| can_prune(entry, now))
                .min_by_key(|(_, entry)| retention_timestamp(entry))
                .map(|(index, _)| index)
            else {
                break;
            };
            self.direct_requests.remove(index);
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
}

fn can_prune(entry: &DirectRequestEntry, now: i64) -> bool {
    if retention_timestamp(entry) > now || !entry.pending_outboxes(now).is_empty() {
        return false;
    }
    match entry.record.decision.state {
        DirectDecisionState::Rejected | DirectDecisionState::Revoked => {
            entry.decision_receipt.is_some()
        }
        DirectDecisionState::Failed | DirectDecisionState::Expired => true,
        DirectDecisionState::Pending | DirectDecisionState::Accepted => false,
    }
}

fn retention_timestamp(entry: &DirectRequestEntry) -> i64 {
    entry
        .record
        .delivery
        .changed_at
        .max(entry.record.decision.changed_at)
        .max(entry.record.decision_delivery.changed_at)
}
