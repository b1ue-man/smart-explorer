//! S66: outgoing peer I/O holds no configuration exclusion. Only the durable
//! store call takes a permit and repeats authorization against current pins.
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;

use super::{direct_repair_runtime_guard, SharedDirectRepairRuntimeGuard};
use crate::share::direct_reciprocal_session::DirectRepairSessionError;
use crate::share::direct_reciprocal_store::DirectRepairStoreError;
use crate::share::identity::ShareIdentity;
use crate::share::profiles::{fingerprint_matches, ShareProfiles};
use crate::share::types::{DirectAccessState, DirectContact, PeerEndpoint, ShareAuthState, ShareScope};

pub(crate) struct OutgoingRepairPersistGate {
    pub(crate) transition_slot: Arc<Semaphore>,
    pub(crate) auth: Arc<Mutex<ShareAuthState>>,
    pub(crate) identity: ShareIdentity,
    pub(crate) endpoint: PeerEndpoint,
}

impl OutgoingRepairPersistGate {
    pub(super) async fn acquire(self) -> Result<SharedDirectRepairRuntimeGuard, DirectRepairSessionError> {
        // Opportunistic repair yields to a configuration transition instead
        // of queuing another peer-controlled writer ahead of revocation.
        let transition = self.transition_slot.clone().try_acquire_owned()
            .map_err(|_| DirectRepairSessionError::Store(DirectRepairStoreError::Retryable))?;
        self.authorize_using(ShareProfiles::direct_secret)?;
        Ok(direct_repair_runtime_guard(transition, None))
    }

    fn authorize_using(
        &self,
        secret_for: impl Fn(&DirectContact) -> Option<Vec<u8>>,
    ) -> Result<(), DirectRepairSessionError> {
        let state = self.auth.lock()
            .map_err(|_| DirectRepairSessionError::Store(DirectRepairStoreError::Unavailable))?;
        let local = &state.identity;
        if !state.direct_online
            || local.device_id != self.identity.device_id
            || local.public_key != self.identity.public_key
            || local.node_id != self.identity.node_id
            || local.fingerprint != self.identity.fingerprint
            || local.direct_lookup_id != self.identity.direct_lookup_id
            || state.direct_secret != self.identity.direct_secret()
        {
            return Err(DirectRepairSessionError::PolicyDenied);
        }
        let ShareScope::Direct { contact_id } = &self.endpoint.scope else {
            return Err(DirectRepairSessionError::PolicyDenied);
        };
        let peer = &self.endpoint.presence;
        let authorized = state.direct_contacts.iter().any(|contact| {
            contact.id == *contact_id
                && contact.lookup_id == peer.relation_id
                && contact.access_state == DirectAccessState::Accepted
                && fingerprint_matches(&peer.public_key, &contact.expected_fingerprint)
                && (contact.expected_node_id.is_empty() || contact.expected_node_id == peer.node_id)
                && contact.remote_device_id.as_deref().is_none_or(|id| id == peer.device_id)
                && contact.remote_public_key.as_deref().is_none_or(|key| key == peer.public_key)
                && contact.accepted_public_key.as_deref().is_none_or(|key| key == peer.public_key)
                && secret_for(contact)
                    .is_some_and(|secret| secret == self.endpoint.relation_secret)
        });
        if authorized {
            Ok(())
        } else {
            Err(DirectRepairSessionError::PolicyDenied)
        }
    }
}

#[cfg(test)]
#[path = "direct_reciprocal_outgoing_gate_task_tests.rs"]
mod task_tests;
