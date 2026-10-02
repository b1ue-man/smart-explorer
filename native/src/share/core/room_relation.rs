use std::fmt;

use serde::{Deserialize, Serialize};

use super::exec_policy::ExecGrant;
use super::types::{RoomMember, RoomProfile, ShareStatus};

pub const ROOM_RELATION_SECRET_BYTES: usize = 32;
pub const MAX_ROOM_RELATION_ID_BYTES: usize = 128;
pub const MAX_ROOM_DISPLAY_NAME_BYTES: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub struct RoomRelationMaterial {
    room_id: String,
    secret: [u8; ROOM_RELATION_SECRET_BYTES],
}

impl RoomRelationMaterial {
    pub fn new(room_id: impl Into<String>, mut secret: Vec<u8>) -> Result<Self, RoomRelationError> {
        let room_id = room_id.into();
        if !valid_identifier(&room_id) {
            secret.fill(0);
            return Err(RoomRelationError::InvalidRoomId);
        }
        if secret.len() != ROOM_RELATION_SECRET_BYTES {
            secret.fill(0);
            return Err(RoomRelationError::InvalidSecretLength);
        }
        let mut durable_secret = [0; ROOM_RELATION_SECRET_BYTES];
        durable_secret.copy_from_slice(&secret);
        secret.fill(0);
        Ok(Self {
            room_id,
            secret: durable_secret,
        })
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub(crate) fn secret(&self) -> &[u8] {
        &self.secret
    }
}

impl fmt::Debug for RoomRelationMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomRelationMaterial")
            .field("room_id", &self.room_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for RoomRelationMaterial {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoomJoinIntent;

#[derive(Clone, PartialEq, Eq)]
pub struct RoomRelationOffer {
    display_name: String,
    material: RoomRelationMaterial,
}

impl RoomRelationOffer {
    pub fn new(
        material: RoomRelationMaterial,
        display_name: impl Into<String>,
    ) -> Result<Self, RoomRelationError> {
        let display_name = display_name.into();
        let display_name = canonical_room_display_name(&display_name)?;
        Ok(Self {
            display_name,
            material,
        })
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn material(&self) -> &RoomRelationMaterial {
        &self.material
    }
}

impl fmt::Debug for RoomRelationOffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomRelationOffer")
            .field("display_name", &self.display_name)
            .field("material", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RoomRelationSnapshot {
    room_profile_id: String,
    offer: RoomRelationOffer,
}

impl RoomRelationSnapshot {
    pub fn new(
        room_profile_id: impl Into<String>,
        offer: RoomRelationOffer,
    ) -> Result<Self, RoomRelationError> {
        let room_profile_id = room_profile_id.into();
        if !valid_identifier(&room_profile_id) {
            return Err(RoomRelationError::InvalidProfileId);
        }
        Ok(Self {
            room_profile_id,
            offer,
        })
    }

    pub fn room_profile_id(&self) -> &str {
        &self.room_profile_id
    }

    pub fn offer(&self) -> &RoomRelationOffer {
        &self.offer
    }

    pub fn into_offer(self) -> RoomRelationOffer {
        self.offer
    }
}

impl fmt::Debug for RoomRelationSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomRelationSnapshot")
            .field("room_profile_id", &self.room_profile_id)
            .field("offer", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomPersistenceOutcome {
    Changed { room_profile_id: String },
    AlreadyComplete { room_profile_id: String },
}

impl RoomPersistenceOutcome {
    pub fn room_profile_id(&self) -> &str {
        match self {
            Self::Changed { room_profile_id } | Self::AlreadyComplete { room_profile_id } => {
                room_profile_id
            }
        }
    }

    pub fn changed(&self) -> bool {
        matches!(self, Self::Changed { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomRelationError {
    InvalidRoomId,
    InvalidProfileId,
    InvalidDisplayName,
    InvalidSecretLength,
}

impl fmt::Display for RoomRelationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRoomId => "invalid Room relation identifier",
            Self::InvalidProfileId => "invalid local Room profile identifier",
            Self::InvalidDisplayName => "invalid Room display name",
            Self::InvalidSecretLength => "Room relation secret must contain 32 bytes",
        })
    }
}

impl std::error::Error for RoomRelationError {}

pub(crate) fn canonical_room_display_name(value: &str) -> Result<String, RoomRelationError> {
    let trimmed = value.trim();
    let display_name = if trimmed.is_empty() { "Raum" } else { trimmed };
    if display_name.len() > MAX_ROOM_DISPLAY_NAME_BYTES
        || display_name.chars().any(char::is_control)
    {
        return Err(RoomRelationError::InvalidDisplayName);
    }
    Ok(display_name.to_string())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ROOM_RELATION_ID_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

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
        let Some(member) = self
            .members
            .iter_mut()
            .find(|member| member.device_id == device_id)
        else {
            return false;
        };
        if blocked {
            if member.blocked && member.relation.admission == RoomMemberAdmission::Admitted {
                return false;
            }
            member.blocked = true;
            member.relation.admission = RoomMemberAdmission::Admitted;
            member.exec.disable_without_decision(now);
            self.policy.confirm_new_members = true;
            return true;
        }
        if !member.blocked && member.relation.admission == RoomMemberAdmission::Admitted {
            return false;
        }
        member.blocked = false;
        member.relation.admission = RoomMemberAdmission::Admitted;
        true
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
                    let mut added = member.clone();
                    added.exec = ExecGrant::default();
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
