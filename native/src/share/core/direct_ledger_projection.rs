use super::direct_ledger::DirectRequestDirection;
use super::direct_protocol::{DirectDecisionKind, SignedDirectDecision};
use super::profiles::ShareProfiles;
use super::types::{DirectAccessState, DirectGrant, DirectGrantState, ShareStatus};

impl ShareProfiles {
    pub(crate) fn reset_outgoing_authorization_after_identity_replacement(&mut self) -> usize {
        let mut changed = 0;
        for contact in &mut self.direct_contacts {
            let already_reset = contact.access_state == DirectAccessState::Pending
                && contact.status == ShareStatus::WaitingForAccess
                && contact.request_sent_at.is_none()
                && contact.accepted_at.is_none()
                && contact.accepted_public_key.is_none()
                && contact.last_error.is_none();
            if already_reset {
                continue;
            }
            contact.access_state = DirectAccessState::Pending;
            contact.status = ShareStatus::WaitingForAccess;
            contact.request_sent_at = None;
            contact.accepted_at = None;
            contact.accepted_public_key = None;
            contact.last_error = None;
            changed += 1;
        }
        changed
    }

    pub(super) fn project_decision(&mut self, index: usize, decision: &SignedDirectDecision) {
        match self.direct_requests[index].direction {
            DirectRequestDirection::Incoming => self.project_grant(decision),
            DirectRequestDirection::Outgoing => {
                if let Some(contact_id) = self.direct_requests[index].contact_id.clone() {
                    self.project_outgoing_decision(&contact_id, decision);
                }
            }
        }
    }

    pub(super) fn project_outgoing_decision(
        &mut self,
        contact_id: &str,
        decision: &SignedDirectDecision,
    ) {
        let Some(contact) = self
            .direct_contacts
            .iter_mut()
            .find(|contact| contact.id == contact_id)
        else {
            return;
        };
        contact.remote_device_id = Some(decision.target.device_id.clone());
        contact.remote_public_key = Some(decision.target.public_key.clone());
        contact.last_error = None;
        match decision.decision {
            DirectDecisionKind::Accepted => {
                contact.access_state = DirectAccessState::Accepted;
                contact.accepted_at = Some(decision.decided_at);
                contact.accepted_public_key = Some(decision.target.public_key.clone());
            }
            DirectDecisionKind::Rejected | DirectDecisionKind::Revoked => {
                contact.access_state = DirectAccessState::Ignored;
                contact.accepted_at = None;
                contact.accepted_public_key = None;
                contact.status = ShareStatus::WaitingForAccess;
            }
        }
    }

    fn project_grant(&mut self, decision: &SignedDirectDecision) {
        let peer = &decision.requester;
        let device_id = peer.device_id.clone();
        let state = if decision.decision == DirectDecisionKind::Accepted {
            DirectGrantState::Accepted
        } else {
            DirectGrantState::Ignored
        };
        let mut matched_device = false;
        if let Some(grant) = self
            .direct_grants
            .iter_mut()
            .find(|grant| grant.device_id == peer.device_id)
        {
            matched_device = true;
            let identity_changed = grant.node_id != peer.node_id
                || grant.public_key != peer.public_key
                || grant.fingerprint != peer.fingerprint;
            let may_replace_inactive = decision.decision == DirectDecisionKind::Accepted
                && grant.state != DirectGrantState::Accepted;
            if !identity_changed || may_replace_inactive {
                if identity_changed {
                    grant.exec.reset_for_identity_change(decision.decided_at);
                } else if decision.decision != DirectDecisionKind::Accepted {
                    grant.exec.disable_for_base_decision(
                        decision.decided_at,
                        decision.request_id.clone(),
                        decision.decision_revision,
                    );
                }
                grant.device_name = peer.device_name.clone();
                grant.node_id = peer.node_id.clone();
                grant.public_key = peer.public_key.clone();
                grant.fingerprint = peer.fingerprint.clone();
                grant.state = state.clone();
                grant.updated_at = decision.decided_at;
            }
            // Otherwise a reject/revoke for a spoofed same-device identity
            // records request history without altering the legitimate grant.
        }
        if !matched_device {
            let mut exec = super::exec_policy::ExecGrant::default();
            if decision.decision != DirectDecisionKind::Accepted {
                exec.disable_for_base_decision(
                    decision.decided_at,
                    decision.request_id.clone(),
                    decision.decision_revision,
                );
            }
            self.direct_grants.push(DirectGrant {
                device_id: peer.device_id.clone(),
                device_name: peer.device_name.clone(),
                node_id: peer.node_id.clone(),
                public_key: peer.public_key.clone(),
                fingerprint: peer.fingerprint.clone(),
                state,
                updated_at: decision.decided_at,
                exec,
            });
        }
        self.recompute_identity_conflicts_for_device(&device_id);
        if decision.decision != DirectDecisionKind::Accepted {
            self.mark_legacy_revoked_for_peer(peer, decision.decided_at);
        }
    }
}
