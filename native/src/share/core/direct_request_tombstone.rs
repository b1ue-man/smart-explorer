use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::direct_ledger::{DirectLedgerError, DirectRequestDirection, DirectRequestEntry};
use super::direct_lifecycle::DirectDecisionState;
use super::direct_protocol::{
    DirectDecisionKind, DirectRequestId, SignedDirectDecision, SignedDirectRequest,
};
use super::profiles::ShareProfiles;

/// Tombstones are small, but unexpired entries are security state and may not
/// be evicted just to make room. A saturated ledger rejects another deletion
/// before changing the visible request ledger.
pub(crate) const MAX_DIRECT_REQUEST_TOMBSTONES: usize = 64;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DirectRequestDeleteDisposition {
    IncomingDismissed,
    OutgoingCancelled,
    HistoryDeleted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectRequestTombstone {
    pub(crate) request: SignedDirectRequest,
    pub(crate) direction: DirectRequestDirection,
    pub(crate) contact_id: Option<String>,
    pub(crate) decision_state: DirectDecisionState,
    pub(crate) decision_revision: u64,
    pub(crate) deleted_at: i64,
    pub(crate) retain_until: i64,
    pub(crate) disposition: DirectRequestDeleteDisposition,
}

impl ShareProfiles {
    pub(crate) fn direct_request_tombstone(
        &self,
        request_id: &DirectRequestId,
    ) -> Option<&DirectRequestTombstone> {
        self.direct_request_tombstones
            .iter()
            .find(|tombstone| tombstone.request.request_id == *request_id)
    }

    pub(crate) fn delete_direct_request_locally(
        &mut self,
        request_id: &DirectRequestId,
        now: i64,
    ) -> Result<bool, DirectLedgerError> {
        if now < 0 {
            return Err(DirectLedgerError::InvalidTimestamp);
        }
        let Some(index) = self
            .direct_requests
            .iter()
            .position(|entry| entry.record.request.request_id == *request_id)
        else {
            return if self.direct_request_tombstone(request_id).is_some() {
                Ok(false)
            } else {
                Err(DirectLedgerError::UnknownRequest)
            };
        };
        let entry = self.direct_requests[index].clone();
        if entry.direction == DirectRequestDirection::Incoming
            && entry.record.decision.state != DirectDecisionState::Pending
            && !entry.removable_from_history(now)
        {
            return Err(
                if entry.record.decision.state == DirectDecisionState::Accepted {
                    DirectLedgerError::ActiveGrantRequiresRevoke
                } else {
                    DirectLedgerError::PendingPeerDelivery
                },
            );
        }
        let tombstone = tombstone_for(&entry, now)?;
        self.prune_direct_request_tombstones(now);
        if self.direct_request_tombstones.len() >= MAX_DIRECT_REQUEST_TOMBSTONES {
            return Err(DirectLedgerError::TombstoneFull);
        }
        self.direct_request_tombstones.push(tombstone);
        self.direct_requests.remove(index);
        Ok(true)
    }

    pub(crate) fn tombstone_blocks_request(
        &self,
        request: &SignedDirectRequest,
        direction: DirectRequestDirection,
    ) -> Result<bool, DirectLedgerError> {
        let Some(tombstone) = self.direct_request_tombstone(&request.request_id) else {
            return Ok(false);
        };
        if tombstone.direction != direction || tombstone.request.digest()? != request.digest()? {
            return Err(DirectLedgerError::RequestIdConflict);
        }
        Ok(true)
    }

    pub(crate) fn tombstone_blocks_digest(
        &self,
        request_id: &DirectRequestId,
        request_digest: &str,
    ) -> Result<bool, DirectLedgerError> {
        let Some(tombstone) = self.direct_request_tombstone(request_id) else {
            return Ok(false);
        };
        if tombstone.request.digest()? != request_digest {
            return Err(DirectLedgerError::RequestIdConflict);
        }
        Ok(true)
    }

    pub(crate) fn tombstoned_outgoing_request(
        &self,
        request_id: &DirectRequestId,
    ) -> Option<(&SignedDirectRequest, &str)> {
        let tombstone = self.direct_request_tombstone(request_id)?;
        if tombstone.direction != DirectRequestDirection::Outgoing {
            return None;
        }
        Some((&tombstone.request, tombstone.contact_id.as_deref()?))
    }

    pub(crate) fn record_tombstoned_direct_decision(
        &mut self,
        decision: &SignedDirectDecision,
    ) -> Result<Option<bool>, DirectLedgerError> {
        let Some(index) = self
            .direct_request_tombstones
            .iter()
            .position(|tombstone| tombstone.request.request_id == decision.request_id)
        else {
            return Ok(None);
        };
        let tombstone = &self.direct_request_tombstones[index];
        super::direct_ledger_mutations::require_decision(&tombstone.request, decision)?;
        let allow_revoke = tombstone.direction == DirectRequestDirection::Outgoing
            && tombstone.decision_state == DirectDecisionState::Accepted
            && decision.decision == DirectDecisionKind::Revoked
            && decision.decision_revision > tombstone.decision_revision;
        if !allow_revoke {
            return Ok(Some(false));
        }
        let contact_id = tombstone.contact_id.clone();
        let tombstone = &mut self.direct_request_tombstones[index];
        tombstone.decision_state = DirectDecisionState::Revoked;
        tombstone.decision_revision = decision.decision_revision;
        if let Some(contact_id) = contact_id {
            self.project_outgoing_decision(&contact_id, decision);
        }
        Ok(Some(true))
    }

    pub(crate) fn prune_direct_request_tombstones(&mut self, now: i64) -> usize {
        let before = self.direct_request_tombstones.len();
        self.direct_request_tombstones
            .retain(|tombstone| tombstone.retain_until >= now);
        before - self.direct_request_tombstones.len()
    }

    pub(super) fn validate_direct_request_tombstones(&self) -> Result<(), String> {
        if self.direct_request_tombstones.len() > MAX_DIRECT_REQUEST_TOMBSTONES {
            return Err(format!(
                "zu viele geloeschte direkte Requests (maximal {MAX_DIRECT_REQUEST_TOMBSTONES})"
            ));
        }
        let active = self
            .direct_requests
            .iter()
            .map(|entry| entry.record.request.request_id.as_str())
            .collect::<HashSet<_>>();
        let mut tombstoned = HashSet::with_capacity(self.direct_request_tombstones.len());
        for tombstone in &self.direct_request_tombstones {
            let id = tombstone.request.request_id.as_str();
            if active.contains(id) || !tombstoned.insert(id) {
                return Err(format!("doppelte direkte Request-ID {id}"));
            }
            let disposition_valid = match tombstone.disposition {
                DirectRequestDeleteDisposition::IncomingDismissed => {
                    tombstone.direction == DirectRequestDirection::Incoming
                        && tombstone.decision_state == DirectDecisionState::Pending
                }
                DirectRequestDeleteDisposition::OutgoingCancelled => {
                    tombstone.direction == DirectRequestDirection::Outgoing
                        && tombstone.decision_state == DirectDecisionState::Pending
                }
                DirectRequestDeleteDisposition::HistoryDeleted => {
                    tombstone.decision_state != DirectDecisionState::Pending
                }
            };
            if tombstone.request.digest().is_err()
                || tombstone.deleted_at < 0
                || tombstone.retain_until < tombstone.deleted_at
                || (tombstone.decision_state == DirectDecisionState::Pending)
                    != (tombstone.decision_revision == 0)
                || !disposition_valid
                || (tombstone.direction == DirectRequestDirection::Outgoing
                    && tombstone.contact_id.as_deref().is_none_or(str::is_empty))
                || (tombstone.direction == DirectRequestDirection::Incoming
                    && tombstone.contact_id.is_some())
            {
                return Err(format!("ungueltiger geloeschter direkter Request {id}"));
            }
        }
        Ok(())
    }
}

fn tombstone_for(
    entry: &DirectRequestEntry,
    now: i64,
) -> Result<DirectRequestTombstone, DirectLedgerError> {
    let disposition = match (entry.direction, entry.record.decision.state) {
        (DirectRequestDirection::Incoming, DirectDecisionState::Pending) => {
            DirectRequestDeleteDisposition::IncomingDismissed
        }
        (DirectRequestDirection::Outgoing, DirectDecisionState::Pending) => {
            DirectRequestDeleteDisposition::OutgoingCancelled
        }
        _ => DirectRequestDeleteDisposition::HistoryDeleted,
    };
    Ok(DirectRequestTombstone {
        request: entry.record.request.clone(),
        direction: entry.direction,
        contact_id: entry.contact_id.clone(),
        decision_state: entry.record.decision.state,
        decision_revision: entry.record.decision.revision,
        deleted_at: now,
        retain_until: if entry.direction == DirectRequestDirection::Outgoing
            && entry.record.decision.state == DirectDecisionState::Accepted
        {
            i64::MAX
        } else {
            entry.record.request.expires_at.max(now)
        },
        disposition,
    })
}

#[cfg(test)]
#[path = "direct_request_tombstone_tests.rs"]
mod tests;
