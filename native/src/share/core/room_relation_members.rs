//! Room members and room-wide rights (contract V5): write right and
//! admission policy (FC1, B15), member upsert from verified presences with
//! key-based blocks (S22), and the runtime half of a room (FA3).
use serde::{Deserialize, Serialize};

use super::super::exec_policy::ExecGrant;
use super::super::types::{PeerPresence, RoomMember, RoomProfile, ShareStatus};

/// Room-wide rights and admission policy (contract V5, FC1, B15).
///
/// Deliberately without `Default`: a new room takes `RoomPolicy::new_room`,
/// a room persisted before V5 deserializes as `RoomPolicy::legacy`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomPolicy {
    /// FC1 „Darf schreiben“ for the members of this room: they may write where
    /// a room export is read-write. Off for new rooms; rooms persisted before
    /// V5 keep it.
    #[serde(default)]
    pub members_may_write: bool,
    /// B15 „Neue Mitglieder bestätigen“: identities seen for the first time
    /// stay pending until the user admits them. Switched on by the first
    /// block in this room (hint: „Raum neu anlegen empfohlen“).
    #[serde(default)]
    pub confirm_new_members: bool,
}

impl RoomPolicy {
    /// Policy of a room created or joined from now on (by code, PIN or UI).
    pub fn new_room() -> Self {
        Self {
            members_may_write: false,
            confirm_new_members: false,
        }
    }

    /// Policy assumed for rooms persisted before V5.
    pub(crate) fn legacy() -> Self {
        Self {
            members_may_write: true,
            confirm_new_members: false,
        }
    }
}

/// Whether a room member may use the relation (B15).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoomMemberAdmission {
    #[default]
    Admitted,
    /// Seen while the room confirms new members. Invariant: a pending member
    /// is also `blocked`, so every check of `blocked` already denies it.
    Pending,
}

/// Per-member relation state and learned security facts (V5).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomMemberFlags {
    #[serde(default)]
    pub admission: RoomMemberAdmission,
    /// B03: the member delivered a presence signed with its pinned key;
    /// unsigned presences for this member are rejected from then on.
    #[serde(default)]
    pub signed_presence: bool,
}

impl RoomMember {
    /// Gate for sessions, opening, Exec and listings: neither blocked by the
    /// user nor waiting for admission.
    pub(crate) fn is_admitted(&self) -> bool {
        !self.blocked && self.relation.admission == RoomMemberAdmission::Admitted
    }

    /// Whether this member authorizes an incoming room session of exactly
    /// this identity. The caller still checks the fingerprint and the proof.
    pub(crate) fn authorizes_session(
        &self,
        device_id: &str,
        public_key: &str,
        node_id: &str,
    ) -> bool {
        self.is_admitted()
            && self.device_id == device_id
            && self.public_key == public_key
            && self.node_id == node_id
    }
}

impl RoomProfile {
    /// New identities need the user's admission: the room asks for it or a
    /// member was blocked by the user (also before V5).
    pub(crate) fn requires_member_confirmation(&self) -> bool {
        self.policy.confirm_new_members
            || self.members.iter().any(|member| {
                member.blocked && member.relation.admission == RoomMemberAdmission::Admitted
            })
    }

    /// User block of one member. Blocking turns its Exec off and switches the
    /// room to „Neue Mitglieder bestätigen“ (B15); unblocking also admits a
    /// pending member. Returns whether anything changed.
    pub fn set_member_blocked(&mut self, device_id: &str, blocked: bool, now: i64) -> bool {
        let Some(principal) = self
            .members
            .iter()
            .find(|member| member.device_id == device_id)
        else {
            return false;
        };
        let public_key = principal.public_key.clone();
        let node_id = principal.node_id.clone();
        let mut changed = blocked && !self.policy.confirm_new_members;
        if blocked {
            self.policy.confirm_new_members = true;
        }
        for member in &mut self.members {
            if member.device_id != device_id
                && !same(&public_key, &member.public_key)
                && !same(&node_id, &member.node_id)
            {
                continue;
            }
            if member.blocked != blocked
                || member.relation.admission != RoomMemberAdmission::Admitted
                || member.exec.enabled && blocked
            {
                member.blocked = blocked;
                member.relation.admission = RoomMemberAdmission::Admitted;
                member.exec.disable_without_decision(now);
                changed = true;
            }
        }
        changed
    }

    /// B15 „Zulassen“ for a pending member. Returns whether it was pending.
    pub fn admit_member(&mut self, device_id: &str) -> bool {
        let Some(member) = self.members.iter_mut().find(|member| {
            member.device_id == device_id
                && member.relation.admission == RoomMemberAdmission::Pending
        }) else {
            return false;
        };
        member.blocked = false;
        member.relation.admission = RoomMemberAdmission::Admitted;
        if member.exec.enabled {
            member.exec.disable_without_decision(member.exec.changed_at);
        }
        true
    }
}

/// FA3: runtime half of one room, forwarded without a configuration
/// transition (`ShareCmd::UpdateRuntime`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomRuntime {
    pub room_id: String,
    pub status: ShareStatus,
    pub last_seen: Option<i64>,
    /// Members as recorded from verified presences. Known members (same
    /// device, key and node) take only runtime fields; unknown ones are added
    /// with Exec off; members with another identity are left alone.
    pub members: Vec<RoomMember>,
}

impl RoomRuntime {
    pub(crate) fn of(room: &RoomProfile) -> Self {
        Self {
            room_id: room.room_id.clone(),
            status: room.status.clone(),
            last_seen: room.last_seen,
            members: room.members.clone(),
        }
    }
}

/// Applies runtime updates to the rooms with the same `room_id`. Never
/// changes exports, policy, `auto_join` or a known member's rights.
pub(crate) fn apply_room_runtime(rooms: &mut [RoomProfile], runtime: &[RoomRuntime]) -> bool {
    let mut changed = false;
    for update in runtime {
        let Some(room) = rooms.iter_mut().find(|room| room.room_id == update.room_id) else {
            continue;
        };
        if room.status != update.status || room.last_seen != update.last_seen {
            room.status = update.status.clone();
            room.last_seen = update.last_seen;
            changed = true;
        }
        for member in &update.members {
            match room
                .members
                .iter_mut()
                .find(|known| known.device_id == member.device_id)
            {
                Some(known) => {
                    if known.public_key == member.public_key && known.node_id == member.node_id {
                        changed |= copy_member_runtime(known, member);
                    }
                }
                None => {
                    if room.members.iter().any(|known| {
                        same(&known.public_key, &member.public_key)
                            || same(&known.node_id, &member.node_id)
                    }) {
                        continue;
                    }
                    let mut added = member.clone();
                    added.exec = ExecGrant::default();
                    if room.requires_member_confirmation() {
                        added.blocked = true;
                        added.relation.admission = RoomMemberAdmission::Pending;
                    }
                    if added.relation.admission == RoomMemberAdmission::Pending {
                        added.blocked = true;
                        room.drop_oldest_pending_beyond(MAX_PENDING_ROOM_MEMBERS.saturating_sub(1));
                    }
                    room.members.push(added);
                    changed = true;
                }
            }
        }
    }
    changed
}

fn copy_member_runtime(known: &mut RoomMember, update: &RoomMember) -> bool {
    if known.device_name == update.device_name
        && known.relay_url == update.relay_url
        && known.candidates == update.candidates
        && known.last_seen == update.last_seen
        && known.status == update.status
        && known.presence == update.presence
    {
        return false;
    }
    known.device_name = update.device_name.clone();
    known.relay_url = update.relay_url.clone();
    known.candidates = update.candidates.clone();
    known.last_seen = update.last_seen;
    known.status = update.status.clone();
    known.presence = update.presence.clone();
    true
}

/// Pending identities kept per room. Pending entries come from members holding
/// the room secret; the bound keeps the profile and its IPC snapshot small
/// when such a member mints identities, and the newest ones stay visible.
pub const MAX_PENDING_ROOM_MEMBERS: usize = 32;

/// What a verified room presence did to the member list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberUpsert {
    /// Runtime data of a known member.
    Updated,
    /// A known member's empty node pin was filled or its first valid
    /// signature remembered.
    Pinned,
    /// A new member, admitted.
    Added,
    /// A new identity waiting for the user's admission (B15).
    Pending,
    /// The presence contradicts the pinned key or node of its device id: it is
    /// left out, the member's rights stay (S22).
    Conflict,
    /// The key or node belongs to another (or a blocked) member: never a new
    /// member (S22).
    Refused,
}

impl RoomProfile {
    /// Applies a presence the worker verified (RoomRoster/RoomJoined) for a
    /// device other than this one.
    pub fn upsert_member_from_presence(
        &mut self,
        presence: PeerPresence,
        now: i64,
    ) -> MemberUpsert {
        if let Some(member) = self
            .members
            .iter_mut()
            .find(|member| member.device_id == presence.device_id)
        {
            return update_member(member, presence, now);
        }
        let known_key = self.members.iter().any(|member| {
            same(&member.public_key, &presence.public_key)
                || same(&member.node_id, &presence.node_id)
        });
        if known_key {
            return MemberUpsert::Refused;
        }
        let pending = self.requires_member_confirmation();
        if pending {
            self.drop_oldest_pending_beyond(MAX_PENDING_ROOM_MEMBERS.saturating_sub(1));
        }
        self.members.push(RoomMember {
            device_id: presence.device_id.clone(),
            device_name: presence.device_name.clone(),
            fingerprint: presence.fingerprint.clone(),
            public_key: presence.public_key.clone(),
            node_id: presence.node_id.clone(),
            relay_url: presence.relay_url.clone(),
            candidates: presence.candidates.clone(),
            last_seen: Some(now),
            status: ShareStatus::Available,
            blocked: pending,
            exec: ExecGrant::default(),
            relation: RoomMemberFlags {
                admission: if pending {
                    RoomMemberAdmission::Pending
                } else {
                    RoomMemberAdmission::Admitted
                },
                signed_presence: presence.is_signed(),
            },
            presence: Some(presence),
        });
        if pending {
            MemberUpsert::Pending
        } else {
            MemberUpsert::Added
        }
    }

    fn drop_oldest_pending_beyond(&mut self, keep: usize) {
        while self
            .members
            .iter()
            .filter(|member| member.relation.admission == RoomMemberAdmission::Pending)
            .count()
            > keep
        {
            let Some(index) = self
                .members
                .iter()
                .enumerate()
                .filter(|(_, member)| member.relation.admission == RoomMemberAdmission::Pending)
                .min_by_key(|(_, member)| member.last_seen.unwrap_or(i64::MIN))
                .map(|(index, _)| index)
            else {
                return;
            };
            self.members.remove(index);
        }
    }
}

fn update_member(member: &mut RoomMember, presence: PeerPresence, now: i64) -> MemberUpsert {
    if member.public_key != presence.public_key
        || (!member.node_id.is_empty() && member.node_id != presence.node_id)
    {
        member.status = ShareStatus::IdentityConflict;
        return MemberUpsert::Conflict;
    }
    let mut pinned = false;
    if member.node_id.is_empty() {
        member.node_id = presence.node_id.clone();
        pinned = true;
    }
    if presence.is_signed() && !member.relation.signed_presence {
        member.relation.signed_presence = true;
        pinned = true;
    }
    member.device_name = presence.device_name.clone();
    member.candidates = presence.candidates.clone();
    member.relay_url = presence.relay_url.clone();
    member.last_seen = Some(now);
    member.status = ShareStatus::Available;
    member.presence = Some(presence);
    if pinned {
        MemberUpsert::Pinned
    } else {
        MemberUpsert::Updated
    }
}

fn same(left: &str, right: &str) -> bool {
    !left.is_empty() && left == right
}
