//! Review 2, follow-up of W8 (2026-10-31), rooms and the guards around the
//! records: the rights chosen for a room and its member access reach the
//! members' grants only through the Exec-grant journal, a record that can
//! never be applied is dropped, records raise the profile version, and the
//! source holds no other writer of commands, terminal or screen.
use super::abgleich_task_tests::{accept, host, journal_step, NOW};
use super::*;
use crate::share::access_grant::Capability;
use crate::share::connection_rights::{ConnectionRights, RightsPreset, RightsTarget};
use crate::share::{
    DirectPeerIdentity, RoomMember, RoomMemberAdmission, RoomPolicy, RoomProfile, ShareStatus,
};

fn member(device: &str, seed: u8) -> RoomMember {
    let pins =
        DirectPeerIdentity::from_secret(device, device, &iroh::SecretKey::from_bytes(&[seed; 32]));
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
        exec: Default::default(),
        presence: None,
        relation: Default::default(),
    };
    member.relation.admission = RoomMemberAdmission::Admitted;
    member
}

fn room_with(members: Vec<RoomMember>) -> RoomProfile {
    RoomProfile {
        id: "room-1".into(),
        name: "Team".into(),
        room_id: "rid".into(),
        auto_join: true,
        last_seen: None,
        status: Default::default(),
        members,
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    }
}

#[test]
fn abgleich_task_follow_up_room_rights_reach_members_only_through_the_journal() {
    let mut profiles = host();
    profiles.rooms.push(room_with(vec![member("m1", 11)]));
    profiles
        .apply_room_connection_rights_deferring_exec(
            "room-1",
            &RightsPreset::FilesExec.rights(),
            NOW,
        )
        .unwrap();
    let room = &profiles.rooms[0];
    assert!(room.policy.members_may_write, "write is stored at once");
    assert!(
        !room.policy.member_access.is_empty(),
        "the access kept for later members"
    );
    assert!(
        !room.members[0].exec.grants_any(),
        "the admitted member is not changed"
    );
    assert_eq!(room.policy.member_follow_ups.len(), 1);

    // A member admitted later gets the room's access the same way.
    profiles.rooms[0].members.push(member("m2", 12));
    assert!(profiles.rooms[0].apply_member_access("m2", NOW + 1));
    assert!(!profiles.rooms[0].members[1].exec.grants_any());
    assert_eq!(profiles.rooms[0].policy.member_follow_ups.len(), 2);

    for (done, device) in [("m1", 0), ("m2", 1)] {
        let mutation = journal_step(&mut profiles, NOW + 2);
        let crate::share::ExecGrantTarget::RoomMember {
            room_id, device_id, ..
        } = &mutation.target
        else {
            panic!("a room member");
        };
        assert_eq!((room_id.as_str(), device_id.as_str()), ("rid", done));
        assert!(profiles.rooms[0].members[device]
            .exec
            .decision_at(Capability::ExecCommands, NOW + 2)
            .is_on());
    }
    assert!(!profiles.exec_follow_ups_waiting());
}

#[test]
fn abgleich_task_follow_up_room_creation_without_members_and_cancel() {
    let mut profiles = host();
    profiles.rooms.push(room_with(Vec::new()));
    profiles
        .apply_room_connection_rights_deferring_exec("room-1", &RightsPreset::All.rights(), NOW)
        .unwrap();
    assert!(
        !profiles.exec_follow_ups_waiting(),
        "no admitted member yet"
    );
    assert!(!profiles.rooms[0].apply_member_access("nobody", NOW));
    profiles.rooms[0].members.push(member("m1", 11));
    assert!(profiles.rooms[0].apply_member_access("m1", NOW + 1));
    let room_target = RightsTarget::Room {
        profile_id: "room-1".into(),
    };
    assert!(profiles.cancel_connection_rights(&room_target, NOW + 2));
    assert!(!profiles.exec_follow_ups_waiting());
    assert!(!profiles.rooms[0].members[0].exec.grants_any());
}

/// G3 (review 3): a narrowing of a room member's access is in force at the
/// worker before the journal stores it.
#[test]
fn abgleich_task_follow_up_room_member_narrowing_is_masked_for_the_worker() {
    let mut profiles = host();
    profiles.rooms.push(room_with(vec![member("m1", 11)]));
    profiles
        .apply_room_connection_rights("room-1", &RightsPreset::All.rights(), NOW)
        .unwrap();
    assert!(profiles.rooms[0].members[0].exec.grants_any());
    assert!(profiles.masked_by_narrowing_follow_ups(NOW).is_none());

    profiles
        .apply_room_connection_rights_deferring_exec(
            "room-1",
            &RightsPreset::ReadOnly.rights(),
            NOW + 1,
        )
        .unwrap();
    assert_eq!(profiles.rooms[0].policy.member_follow_ups.len(), 1);
    assert!(
        profiles.rooms[0].members[0].exec.grants_any(),
        "the stored grant waits for the journal"
    );
    let masked = profiles
        .masked_by_narrowing_follow_ups(NOW + 2)
        .expect("the member's access narrows");
    assert!(!masked.rooms[0].members[0].exec.grants_any());
    assert!(profiles.rooms[0].members[0].exec.grants_any());
}

#[test]
fn abgleich_task_follow_up_records_that_can_never_apply_are_obsolete() {
    let mut profiles = host();
    accept(&mut profiles, NOW);
    let grant = profiles.grant_for("peer").unwrap().clone();
    let record = |profiles: &mut ShareProfiles| {
        profiles.defer_direct_exec(
            &grant.device_id,
            &grant.public_key,
            RightsPreset::All.rights().access_change(),
            NOW,
        );
    };
    // Without a fully pinned identity the journal refuses everything.
    profiles.direct_grants[0].node_id = String::new();
    record(&mut profiles);
    let next = profiles.next_exec_follow_up().unwrap();
    assert!(matches!(next.work, FollowUpWork::Obsolete(_)));
    profiles.discard_exec_follow_up(&next.key);
    assert!(profiles.exec_follow_ups.is_empty());

    // A grant that is no longer active.
    profiles.direct_grants[0].node_id = grant.node_id.clone();
    record(&mut profiles);
    assert!(matches!(
        profiles.next_exec_follow_up().unwrap().work,
        FollowUpWork::Apply { .. }
    ));
    profiles.direct_grants[0].state = DirectGrantState::Ignored;
    assert!(matches!(
        profiles.next_exec_follow_up().unwrap().work,
        FollowUpWork::Obsolete(_)
    ));
    profiles.prune_exec_follow_ups();
    assert!(profiles.exec_follow_ups.is_empty(), "pruned with the grant");

    // A blocked room member.
    let mut blocked = member("m1", 11);
    blocked.blocked = true;
    profiles.rooms.push(room_with(vec![blocked]));
    profiles.rooms[0]
        .policy
        .member_follow_ups
        .push(MemberFollowUp {
            device_id: "m1".into(),
            change: RightsPreset::All.rights().access_change(),
        });
    assert!(matches!(
        profiles.next_exec_follow_up().unwrap().work,
        FollowUpWork::Obsolete(_)
    ));
}

#[test]
fn abgleich_task_follow_up_records_raise_the_profile_version() {
    let mut profiles = host();
    assert_eq!(profiles.stored_schema_version(), 8);
    profiles.defer_direct_exec("peer", "key", ConnectionRights::none().access_change(), NOW);
    assert_eq!(
        profiles.stored_schema_version(),
        9,
        "an older version must not read the record as a grant without it"
    );
    let mut room = host();
    room.rooms.push(room_with(Vec::new()));
    assert_eq!(room.stored_schema_version(), 8);
    room.rooms[0].policy.member_follow_ups.push(MemberFollowUp {
        device_id: "m1".into(),
        change: AccessGrantChange::default(),
    });
    assert_eq!(room.stored_schema_version(), 9);
}
