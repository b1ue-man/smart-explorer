use std::collections::{BTreeSet, HashSet};
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use super::core::{eio, now_secs};
use super::direct_reciprocal_coordinator::{DirectReciprocalCoordinator, DirectRepairCandidate};
use super::fs::ShareExportConfig;
use super::node::ShareIrohNode;
use super::profiles::ShareProfiles;
use super::relation_rights::authorization_restrictions;
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
        self.apply(direct, direct_grants, rooms, default_direct_exports, None)
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
        let mut steering_change = None;
        let (restrictions, snapshot) = {
            let mut state = self.auth.lock().map_err(|_| eio("Share-State gesperrt"))?;
            let mut candidate = state.clone();
            candidate.direct_contacts = direct;
            candidate.direct_grants = direct_grants;
            candidate.rooms = rooms;
            candidate.default_direct_exports = default_direct_exports;
            if let Some((direct_requests, tombstones)) = requests {
                candidate.direct_requests = direct_requests;
                candidate.direct_request_tombstones = tombstones;
            }
            // The live snapshot owns replay state and immediately learned
            // signatures. A delayed persisted profile cannot erase either.
            super::authorization_policy::preserve_signature_facts(&state, &mut candidate);
            let steering_before = steering_devices(&state, now_secs());
            let steering_after = steering_devices(&candidate, now_secs());
            steering_change = Some((
                granted_since_start(&candidate, &steering_before, now_secs()),
                !steering_before.is_subset(&steering_after),
                steering_after,
            ));
            let restrictions = authorization_restrictions(&state, &candidate);
            if !restrictions.is_empty() {
                candidate.authorization_epoch = state
                    .authorization_epoch
                    .checked_add(1)
                    .ok_or_else(|| eio("Share authorization epoch exhausted"))?;
            }
            super::exec_grant_runtime::apply_configuration_transition(
                &state,
                &candidate,
                candidate.authorization_epoch,
                self.iroh.exec_registry(),
            )?;
            // The deny barrier precedes publication: a worker cannot take
            // a new scope generation while still holding an old export table.
            self.iroh
                .invalidate_restrictions_at(&restrictions, candidate.authorization_epoch)?;
            self.iroh.access_registry().recheck(&candidate, now_secs());
            *state = candidate;
            (restrictions, state.clone())
        };
        if !restrictions.is_empty() {
            self.direct_requests_sent.clear();
        }
        drop(transition);
        if let Some((granted, revoked, steering)) = steering_change {
            apply_steering_change(granted, revoked, steering);
        }
        if let Some(reciprocal) = self.iroh.direct_repair_coordinator() {
            reciprocal.set_current_generation(snapshot.authorization_epoch);
            schedule_snapshot(&snapshot, &reciprocal);
        }
        Ok(())
    }
}

/// Unix second of this process's first configuration; grants changed
/// before it were applied by an earlier process (T2 covers those).
static PROCESS_START: OnceLock<i64> = OnceLock::new();

fn process_start() -> i64 {
    *PROCESS_START.get_or_init(now_secs)
}

/// Every (device ID, public key) that may steer this screen in an active
/// relation of `state` at `now` (`access_steering::steers`).
pub(super) fn steering_devices(state: &ShareAuthState, now: i64) -> BTreeSet<(String, String)> {
    use super::access_steering::steers;
    let mut devices = BTreeSet::new();
    if state.direct_online {
        for grant in state.direct_grants.iter().filter(|grant| {
            grant.state == super::types::DirectGrantState::Accepted
                && steers(&grant.exec.access_grant(), now)
        }) {
            devices.insert((grant.device_id.clone(), grant.public_key.clone()));
        }
    }
    for room in state.rooms.iter().filter(|room| room.auto_join) {
        for member in room
            .members
            .iter()
            .filter(|member| member.is_admitted() && steers(&member.exec.access_grant(), now))
        {
            devices.insert((member.device_id.clone(), member.public_key.clone()));
        }
    }
    devices
}

/// T1 (design 3.1): a device that did not steer before received the right
/// while this process runs. A grant loaded at start is T2's case, so a
/// restart never repeats an administrator prompt by itself (T-R6).
fn granted_since_start(
    candidate: &ShareAuthState,
    before: &BTreeSet<(String, String)>,
    now: i64,
) -> bool {
    let start = process_start();
    let fresh = |device: &str, key: &str, exec: &super::exec_policy::ExecGrant| {
        exec.changed_at >= start
            && super::access_steering::steers(&exec.access_grant(), now)
            && !before.contains(&(device.to_string(), key.to_string()))
    };
    let direct = candidate.direct_online
        && candidate.direct_grants.iter().any(|grant| {
            grant.state == super::types::DirectGrantState::Accepted
                && fresh(&grant.device_id, &grant.public_key, &grant.exec)
        });
    direct
        || candidate
            .rooms
            .iter()
            .filter(|room| room.auto_join)
            .flat_map(|room| room.members.iter())
            .any(|member| {
                member.is_admitted() && fresh(&member.device_id, &member.public_key, &member.exec)
            })
}

/// Records the steering devices and starts the RDP self-setup (T1) or the
/// rotation of se-managed RDP credentials after a revocation (T-R2).
fn apply_steering_change(granted: bool, revoked: bool, steering: BTreeSet<(String, String)>) {
    super::access_steering::note_steering_devices(steering);
    if cfg!(test) {
        return;
    }
    if granted {
        crate::rdp::host_setup::request(crate::rdp::core::SetupTrigger::Grant);
    }
    if revoked {
        crate::rdp::host_setup::rotate_managed_credentials();
    }
}

/// Service start (design 3.1): the RDP port list is filled once in the
/// background, and T2 runs when a relation steers this screen.
pub(super) fn on_node_start(auth: &Arc<Mutex<ShareAuthState>>, device_id: &str) {
    process_start();
    let steering = match auth.lock() {
        Ok(state) => steering_devices(&state, now_secs()),
        Err(_) => BTreeSet::new(),
    };
    let any = !steering.is_empty();
    super::access_steering::note_steering_devices(steering);
    if cfg!(test) {
        return;
    }
    crate::rdp::host_setup::note_host_device(device_id);
    crate::rdp::host_setup::on_service_start(any);
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
    schedule_snapshot_with(snapshot, reciprocal, ShareProfiles::direct_secret_checked);
}

fn schedule_snapshot_with(
    snapshot: &ShareAuthState,
    reciprocal: &DirectReciprocalCoordinator,
    mut direct_secret: impl FnMut(&DirectContact) -> Result<Option<Vec<u8>>, String>,
) {
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
        let Ok(Some(relation_secret)) = direct_secret(contact) else {
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

#[cfg(test)]
#[path = "configuration_runtime_task_tests.rs"]
mod task_tests;
