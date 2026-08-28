use std::collections::HashSet;
use std::io;
use std::sync::{Arc, Mutex};

use super::authorization_policy::configuration_changed;
use super::core::{eio, now_secs};
use super::direct_reciprocal_coordinator::{
    DirectReciprocalCoordinator, DirectRepairCandidate,
};
use super::fs::ShareExportConfig;
use super::node::ShareIrohNode;
use super::profiles::ShareProfiles;
use super::types::{
    DirectAccessState, DirectContact, DirectGrant, PeerEndpoint, RoomProfile, ShareAuthState,
    ShareScope,
};

pub(super) struct RuntimeConfiguration<'a> {
    pub(super) auth: &'a Arc<Mutex<ShareAuthState>>,
    pub(super) iroh: &'a ShareIrohNode,
    pub(super) direct_requests_sent: &'a mut HashSet<String>,
}

impl RuntimeConfiguration<'_> {
    pub(super) fn apply_profiles(&mut self, profiles: ShareProfiles) -> io::Result<()> {
        let requests = Some((profiles.direct_requests, profiles.direct_request_tombstones));
        self.apply(
            profiles.direct_contacts,
            profiles.direct_grants,
            profiles.rooms,
            profiles.default_direct_exports,
            requests,
        )
    }

    pub(super) fn apply_parts(
        &mut self,
        direct: Vec<DirectContact>,
        direct_grants: Vec<DirectGrant>,
        rooms: Vec<RoomProfile>,
        default_direct_exports: ShareExportConfig,
    ) -> io::Result<()> {
        self.apply(
            direct,
            direct_grants,
            rooms,
            default_direct_exports,
            None,
        )
    }

    fn apply(
        &mut self,
        direct: Vec<DirectContact>,
        direct_grants: Vec<DirectGrant>,
        rooms: Vec<RoomProfile>,
        default_direct_exports: ShareExportConfig,
        requests: Option<(
            Vec<super::direct_ledger::DirectRequestEntry>,
            Vec<super::direct_request_tombstone::DirectRequestTombstone>,
        )>,
    ) -> io::Result<()> {
        let transition = self.iroh.begin_runtime_transition()?;
        let (changed, snapshot) = {
            let mut state = self.auth.lock().map_err(|_| eio("Share-State gesperrt"))?;
            let policy_changed = requests.as_ref().is_some_and(|(entries, tombstones)| {
                state.direct_requests.as_slice() != entries.as_slice()
                    || state.direct_request_tombstones.as_slice() != tombstones.as_slice()
            });
            let changed = policy_changed || configuration_changed(
                &state,
                &direct,
                &direct_grants,
                &rooms,
                &default_direct_exports,
            );
            let mut candidate = state.clone();
            candidate.direct_contacts = direct;
            candidate.direct_grants = direct_grants;
            candidate.rooms = rooms;
            candidate.default_direct_exports = default_direct_exports;
            if let Some((direct_requests, tombstones)) = requests {
                candidate.direct_requests = direct_requests;
                candidate.direct_request_tombstones = tombstones;
            }
            if changed {
                let next_epoch = state
                    .authorization_epoch
                    .checked_add(1)
                    .ok_or_else(|| eio("Share authorization epoch exhausted"))?;
                candidate.authorization_epoch = next_epoch;
                super::exec_grant_runtime::apply_configuration_transition(
                    &state,
                    &candidate,
                    next_epoch,
                    self.iroh.exec_registry(),
                )?;
            }
            *state = candidate;
            (changed, state.clone())
        };
        let invalidation = if changed {
            self.direct_requests_sent.clear();
            self.iroh.invalidate_sessions().map(|_| ())
        } else {
            Ok(())
        };
        drop(transition);
        if let Some(reciprocal) = self.iroh.direct_repair_coordinator() {
            reciprocal.set_current_generation(snapshot.authorization_epoch);
            schedule_snapshot(&snapshot, &reciprocal);
        }
        invalidation
    }
}

pub(super) fn schedule_current(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
) -> io::Result<()> {
    let snapshot = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    if let Some(reciprocal) = iroh.direct_repair_coordinator() {
        reciprocal.set_current_generation(snapshot.authorization_epoch);
        schedule_snapshot(&snapshot, &reciprocal);
    }
    Ok(())
}

fn schedule_snapshot(snapshot: &ShareAuthState, reciprocal: &DirectReciprocalCoordinator) {
    if !snapshot.direct_online {
        return;
    }
    for contact in snapshot.direct_contacts.iter().filter(|contact| {
        contact.auto_connect && contact.access_state == DirectAccessState::Accepted
    }) {
        let Some(presence) = contact
            .presence
            .as_ref()
            .filter(|presence| presence.is_current_at(now_secs()))
            .cloned()
        else {
            continue;
        };
        let Ok(Some(relation_secret)) = ShareProfiles::direct_secret_checked(contact) else {
            continue;
        };
        let expected_node_id = if contact.expected_node_id.is_empty() {
            Some(presence.node_id.clone())
        } else {
            Some(contact.expected_node_id.clone())
        };
        let endpoint = PeerEndpoint {
            label: contact.display_name.clone(),
            scope: ShareScope::Direct {
                contact_id: contact.id.clone(),
            },
            presence,
            relation_secret,
            expected_node_id,
        };
        let Ok(candidate) = DirectRepairCandidate::from_accepted_contact(
            snapshot.authorization_epoch,
            contact,
            endpoint,
            snapshot.identity.clone(),
        ) else {
            continue;
        };
        let _ = reciprocal.schedule(candidate);
    }
}
