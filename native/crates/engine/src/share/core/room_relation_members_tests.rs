//! r-entdeckung-7 (suite-2-entdeckung): at most `MAX_ADMITTED_ROOM_MEMBERS`
//! admitted members per room. The member seen longest ago without state of
//! its own gives way; a returning member is admitted again.
use super::*;
use crate::share::exec_follow_up::MemberFollowUp;

fn room() -> RoomProfile {
    RoomProfile {
        id: "room-profile".into(),
        name: "Room".into(),
        room_id: "room-id".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: Vec::new(),
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    }
}

fn presence(index: usize) -> PeerPresence {
    let key = format!("member-key-{index}");
    PeerPresence {
        kind: "room".into(),
        relation_id: "room-id".into(),
        device_id: format!("member-{index}"),
        device_name: format!("Member {index}"),
        public_key: key.clone(),
        fingerprint: String::new(),
        node_id: key,
        relay_url: String::new(),
        candidates: Vec::new(),
        expires_at: i64::MAX,
        nonce: "nonce".into(),
        proof: String::new(),
    }
}

fn runtime_member(index: usize, last_seen: i64) -> RoomMember {
    let key = format!("member-key-{index}");
    RoomMember {
        device_id: format!("member-{index}"),
        device_name: format!("Member {index}"),
        fingerprint: String::new(),
        public_key: key.clone(),
        node_id: key,
        relay_url: String::new(),
        candidates: Vec::new(),
        last_seen: Some(last_seen),
        status: ShareStatus::Available,
        blocked: false,
        exec: ExecGrant::default(),
        presence: None,
        relation: RoomMemberFlags::default(),
    }
}

fn admitted(room: &RoomProfile) -> usize {
    room.members
        .iter()
        .filter(|member| member.relation.admission == RoomMemberAdmission::Admitted)
        .count()
}

fn has(room: &RoomProfile, index: usize) -> bool {
    room.members
        .iter()
        .any(|member| member.device_id == format!("member-{index}"))
}

/// A room holding exactly `MAX_ADMITTED_ROOM_MEMBERS` members, the n-th
/// last seen at `1000 + n`.
fn full_room() -> RoomProfile {
    let mut room = room();
    for index in 0..MAX_ADMITTED_ROOM_MEMBERS {
        let result = room.upsert_member_from_presence(presence(index), 1_000 + index as i64);
        assert_eq!(result, MemberUpsert::Added, "member {index}");
    }
    assert_eq!(room.members.len(), MAX_ADMITTED_ROOM_MEMBERS);
    room
}

#[test]
fn entdeckung_task_a_room_admits_at_most_1024_members_and_the_oldest_gives_way() {
    let mut room = full_room();
    let newcomer = MAX_ADMITTED_ROOM_MEMBERS;
    assert_eq!(
        room.upsert_member_from_presence(presence(newcomer), 5_000),
        MemberUpsert::Added
    );
    assert_eq!(room.members.len(), MAX_ADMITTED_ROOM_MEMBERS);
    assert!(!has(&room, 0), "the member seen longest ago gave way");
    assert!(has(&room, 1));
    assert!(has(&room, newcomer));

    // The same bound holds for members that arrive with a runtime update.
    let mut rooms = vec![self::room()];
    let update = RoomRuntime {
        room_id: "room-id".into(),
        status: ShareStatus::Available,
        last_seen: Some(1),
        members: (0..=MAX_ADMITTED_ROOM_MEMBERS)
            .map(|index| runtime_member(index, 1_000 + index as i64))
            .collect(),
    };
    assert!(apply_room_runtime(&mut rooms, &[update]));
    assert_eq!(admitted(&rooms[0]), MAX_ADMITTED_ROOM_MEMBERS);
    assert!(!has(&rooms[0], 0));
    assert!(has(&rooms[0], MAX_ADMITTED_ROOM_MEMBERS));
}

#[test]
fn entdeckung_task_members_with_rights_or_a_waiting_follow_up_are_not_displaced() {
    let mut room = full_room();
    // The two members seen longest ago carry state of their own.
    room.members[0].exec.enabled = true;
    assert!(room.members[0].exec.grants_any());
    room.policy.member_follow_ups.push(MemberFollowUp {
        device_id: "member-1".into(),
        change: Default::default(),
    });
    let newcomer = MAX_ADMITTED_ROOM_MEMBERS;
    assert_eq!(
        room.upsert_member_from_presence(presence(newcomer), 5_000),
        MemberUpsert::Added
    );
    assert_eq!(room.members.len(), MAX_ADMITTED_ROOM_MEMBERS);
    assert!(has(&room, 0), "a member with Exec rights stays");
    assert!(has(&room, 1), "a member with a waiting follow-up stays");
    assert!(!has(&room, 2), "the next member seen longest ago gave way");
}

#[test]
fn entdeckung_task_a_blocked_member_stays_and_flooding_cannot_displace_it() {
    let mut room = full_room();
    room.members[0].blocked = true;
    // A user block makes every new identity wait for admission.
    assert!(room.requires_member_confirmation());
    assert_eq!(
        room.upsert_member_from_presence(presence(MAX_ADMITTED_ROOM_MEMBERS), 5_000),
        MemberUpsert::Pending
    );
    assert!(has(&room, 0), "the blocked member stays");
    assert_eq!(admitted(&room), MAX_ADMITTED_ROOM_MEMBERS);
}

#[test]
fn entdeckung_task_a_displaced_member_is_admitted_again_by_its_next_presence() {
    let mut room = full_room();
    assert_eq!(
        room.upsert_member_from_presence(presence(MAX_ADMITTED_ROOM_MEMBERS), 5_000),
        MemberUpsert::Added
    );
    assert!(!has(&room, 0));
    assert_eq!(
        room.upsert_member_from_presence(presence(0), 6_000),
        MemberUpsert::Added
    );
    let returned = room
        .members
        .iter()
        .find(|member| member.device_id == "member-0")
        .expect("the returning member");
    assert!(returned.is_admitted());
    assert_eq!(room.members.len(), MAX_ADMITTED_ROOM_MEMBERS);
}
