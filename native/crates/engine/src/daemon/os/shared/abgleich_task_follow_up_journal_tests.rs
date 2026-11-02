//! Review 2, follow-up of W8 (2026-10-31): the rights that profile
//! transactions outside the Exec-grant journal could only record (an
//! acceptance with rights, the Direct code's rights, a PIN offer's rights,
//! a room's member access, the automatic acceptance) become ordinary
//! single-grant journal entries. A grant is stored before the worker gets
//! it, a revocation reaches the worker before it is stored, storing it ends
//! the record in the same step, a crash between the steps is masked and
//! repeated, and an explicit change of the grant supersedes the record.
use std::cell::RefCell;

use super::super::rights::apply_persisted;
use super::super::{
    decode_entry, drive_steps, mask_pending, JournalEntry, JournalPhase, JOURNAL_VERSION,
};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::connection_rights::{ConnectionRights, RightsPreset, RightsTarget};
use crate::share::exec_follow_up::FollowUpWork;
use crate::share::{
    DirectGrant, DirectGrantState, DirectPeerIdentity, ExecGrant, ExecGrantMutation, RoomMember,
    RoomMemberAdmission, RoomPolicy, RoomProfile, ShareIdentity, ShareProfiles, ShareStatus,
};

fn identity() -> ShareIdentity {
    let iroh_secret = iroh::SecretKey::from_bytes(&[1; 32]);
    let local = DirectPeerIdentity::from_secret("local-device", "Local", &iroh_secret);
    ShareIdentity {
        device_id: local.device_id,
        device_name: local.device_name,
        direct_lookup_id: "local-lookup".into(),
        public_key: local.public_key,
        fingerprint: local.fingerprint,
        node_id: local.node_id,
        iroh_secret,
        direct_secret: [7; 32],
    }
}

/// An accepted, fully pinned Direct grant without any right.
fn host() -> (ShareProfiles, RightsTarget) {
    let peer =
        DirectPeerIdentity::from_secret("peer", "Peer", &iroh::SecretKey::from_bytes(&[5; 32]));
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(DirectGrant {
        device_id: peer.device_id.clone(),
        device_name: peer.device_name.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant::default(),
        write: false,
    });
    let target = RightsTarget::Direct {
        device_id: peer.device_id,
        public_key: peer.public_key,
    };
    (profiles, target)
}

/// The record a profile transaction made for `rights` on the Direct grant.
fn record(profiles: &mut ShareProfiles, target: &RightsTarget, rights: &ConnectionRights) {
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = target
    else {
        unreachable!()
    };
    profiles.defer_direct_exec(device_id, public_key, rights.access_change(), 10);
}

/// The journal entry the daemon writes for the next waiting record.
fn entry_for_next(profiles: &ShareProfiles) -> JournalEntry {
    let next = profiles.next_exec_follow_up().expect("a record waits");
    let FollowUpWork::Apply { target, change } = next.work else {
        panic!("the record must be applicable");
    };
    let mut scratch = profiles.clone();
    let (mutation, expected) =
        ExecGrantMutation::prepare_access(&mut scratch, &identity(), target, &change, None, 20)
            .unwrap();
    JournalEntry::new(mutation, expected).unwrap()
}

/// A journal step that only notes that it ran.
fn step<'a>(
    order: &'a RefCell<Vec<&'static str>>,
    name: &'static str,
) -> impl FnMut() -> Result<(), String> + 'a {
    move || {
        order.borrow_mut().push(name);
        Ok(())
    }
}

fn commands_on(profiles: &ShareProfiles) -> bool {
    profiles.direct_grants[0]
        .exec
        .decision_at(Capability::ExecCommands, 30)
        == CapabilityDecision::Allow
}

#[test]
fn abgleich_task_follow_up_entry_is_a_plain_grant_entry_in_the_phase_of_its_direction() {
    let (mut profiles, target) = host();
    record(&mut profiles, &target, &RightsPreset::All.rights());
    let entry = entry_for_next(&profiles);
    assert_eq!(entry.version, JOURNAL_VERSION, "older daemons read it too");
    assert!(entry.more.is_empty() && entry.rights.is_none());
    assert_eq!(entry.phase, JournalPhase::PendingApply, "stored first");
    let encoded = serde_json::to_vec(&entry).unwrap();
    assert_eq!(decode_entry(&encoded).unwrap(), entry);
    entry.validate().unwrap();

    // A revocation is a pending deny: the worker gets it before the file.
    let mut granted = profiles.clone();
    apply_persisted(&entry, &mut granted, &identity()).unwrap();
    assert!(commands_on(&granted));
    record(&mut granted, &target, &RightsPreset::None.rights());
    let revoke = entry_for_next(&granted);
    assert_eq!(revoke.phase, JournalPhase::PendingDeny, "worker first");
    assert!(!revoke.mutation.policy.grants_any());
}

#[test]
fn abgleich_task_follow_up_steps_run_in_the_order_of_the_phase_and_a_crash_keeps_the_entry() {
    let (mut profiles, target) = host();
    record(&mut profiles, &target, &RightsPreset::FilesExec.rights());
    let grant = entry_for_next(&profiles);
    let order = RefCell::new(Vec::new());
    let progress = drive_steps(
        grant.phase,
        step(&order, "persist"),
        step(&order, "worker"),
        step(&order, "clear"),
    );
    assert!(progress.cleared);
    assert_eq!(*order.borrow(), ["persist", "worker", "clear"]);

    // Revocation: worker, then file, then the entry ends.
    let mut allowed = profiles.clone();
    apply_persisted(&grant, &mut allowed, &identity()).unwrap();
    record(&mut allowed, &target, &RightsPreset::None.rights());
    let revoke = entry_for_next(&allowed);
    order.borrow_mut().clear();
    let progress = drive_steps(
        revoke.phase,
        step(&order, "persist"),
        step(&order, "worker"),
        step(&order, "clear"),
    );
    assert!(progress.cleared);
    assert_eq!(*order.borrow(), ["worker", "persist", "clear"]);

    // A crash after storing a grant, before the worker: the entry stays.
    let crashed = drive_steps(
        grant.phase,
        || Ok(()),
        || Err("worker busy".to_string()),
        || Ok(()),
    );
    assert!(crashed.persisted && !crashed.applied && !crashed.cleared);
    // A revocation the worker did not get is not stored either.
    let refused = drive_steps(
        revoke.phase,
        || panic!("stored before the worker had it"),
        || Err("worker busy".to_string()),
        || Ok(()),
    );
    assert!(!refused.persisted && !refused.cleared);
}

#[test]
fn abgleich_task_follow_up_storing_the_grant_ends_the_record_in_the_same_step() {
    let (mut profiles, target) = host();
    record(&mut profiles, &target, &RightsPreset::All.rights());
    profiles
        .pending_rights
        .push(crate::share::connection_rights::PendingRights {
            target: target.clone(),
            origin: crate::share::connection_rights::RightsOrigin::Request,
            since: 10,
        });
    let entry = entry_for_next(&profiles);
    let mut stored = profiles.clone();
    apply_persisted(&entry, &mut stored, &identity()).unwrap();
    assert!(commands_on(&stored));
    assert!(
        stored.exec_follow_ups.is_empty(),
        "the record ends with the grant"
    );
    assert!(
        stored.pending_rights.is_empty(),
        "so does the open confirmation"
    );
    // A repeat after a crash changes nothing more.
    let again = stored.clone();
    apply_persisted(&entry, &mut stored, &identity()).unwrap();
    assert_eq!(stored.direct_grants, again.direct_grants);
    assert_eq!(stored.exec_follow_ups, again.exec_follow_ups);
    assert_eq!(stored.pending_rights, again.pending_rights);
}

#[test]
fn abgleich_task_follow_up_a_pending_entry_masks_the_grant_until_it_is_repeated() {
    let (mut profiles, target) = host();
    record(&mut profiles, &target, &RightsPreset::All.rights());
    let entry = entry_for_next(&profiles);
    // Crash after the entry was written, before anything was stored.
    let mut reloaded = profiles.clone();
    mask_pending(&mut reloaded, &identity(), &entry).unwrap();
    assert!(!commands_on(&reloaded));
    assert!(
        !reloaded.exec_follow_ups.is_empty(),
        "the record is still there"
    );
    apply_persisted(&entry, &mut reloaded, &identity()).unwrap();
    assert!(commands_on(&reloaded), "the repeat stores it");
    // Crash after storing, before the worker: the reload masks it again.
    let mut masked = reloaded.clone();
    mask_pending(&mut masked, &identity(), &entry).unwrap();
    assert!(!commands_on(&masked));
    apply_persisted(&entry, &mut reloaded, &identity()).unwrap();
    assert!(commands_on(&reloaded));
}

#[test]
fn abgleich_task_follow_up_an_explicit_change_supersedes_the_record_and_the_stale_entry_fails() {
    let (mut profiles, target) = host();
    record(&mut profiles, &target, &RightsPreset::All.rights());
    let stale = entry_for_next(&profiles);

    // The user sets the grant (MutateGrant) before the entry ran.
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = &target
    else {
        unreachable!()
    };
    let grant = profiles.grant_for(device_id).unwrap().clone();
    let exact = crate::share::ExecGrantTarget::Direct {
        device_id: device_id.clone(),
        public_key: public_key.clone(),
        fingerprint: grant.fingerprint,
        node_id: grant.node_id,
    };
    let mut scratch = profiles.clone();
    let (mutation, expected) = ExecGrantMutation::prepare_access(
        &mut scratch,
        &identity(),
        exact,
        &RightsPreset::ReadWrite.rights().access_change(),
        None,
        25,
    )
    .unwrap();
    let explicit = JournalEntry::new(mutation, expected).unwrap();
    let mut stored = profiles.clone();
    apply_persisted(&explicit, &mut stored, &identity()).unwrap();
    assert!(stored.exec_follow_ups.is_empty(), "superseded");
    assert!(!commands_on(&stored));

    // The stale entry would overwrite a newer grant: it is refused.
    let before = stored.clone();
    assert!(apply_persisted(&stale, &mut stored, &identity()).is_err());
    assert_eq!(stored.direct_grants, before.direct_grants);
}

#[test]
fn abgleich_task_follow_up_room_member_access_goes_through_the_same_entry() {
    let pins = DirectPeerIdentity::from_secret("m1", "M1", &iroh::SecretKey::from_bytes(&[11; 32]));
    let mut member = RoomMember {
        device_id: pins.device_id,
        device_name: pins.device_name,
        fingerprint: pins.fingerprint,
        public_key: pins.public_key,
        node_id: pins.node_id,
        relay_url: String::new(),
        candidates: Vec::new(),
        last_seen: None,
        status: ShareStatus::Waiting,
        blocked: false,
        exec: ExecGrant::default(),
        presence: None,
        relation: Default::default(),
    };
    member.relation.admission = RoomMemberAdmission::Admitted;
    let mut profiles = ShareProfiles::default();
    profiles.rooms.push(RoomProfile {
        id: "room-1".into(),
        name: "Team".into(),
        room_id: "rid".into(),
        auto_join: true,
        last_seen: None,
        status: Default::default(),
        members: vec![member],
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    });
    profiles
        .apply_room_connection_rights_deferring_exec(
            "room-1",
            &RightsPreset::FilesExec.rights(),
            10,
        )
        .unwrap();
    assert!(!profiles.rooms[0].members[0].exec.grants_any());
    let entry = entry_for_next(&profiles);
    assert_eq!(entry.phase, JournalPhase::PendingApply);
    assert!(matches!(
        entry.mutation.target,
        crate::share::ExecGrantTarget::RoomMember { .. }
    ));
    apply_persisted(&entry, &mut profiles, &identity()).unwrap();
    assert!(profiles.rooms[0].members[0]
        .exec
        .decision_at(Capability::ExecCommands, 30)
        .is_on());
    assert!(profiles.rooms[0].policy.member_follow_ups.is_empty());
}
