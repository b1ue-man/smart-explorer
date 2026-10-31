//! Commands, terminal and screen of a connection are given or changed only
//! through the Exec-grant journal of the background worker
//! (`daemon/os/shared/exec_grant_journal*.rs`): the journal entry under
//! `exec_grant_lock`, revocations reaching the worker before they are
//! stored and grants after, a crashed step masked and repeated on reload,
//! and the storing in one profile transaction.
//!
//! Every other profile transaction that would give or change those rights
//! (accepting a request with rights, the rights of the Direct code, the
//! rights of a PIN offer, a room's access for its members, the automatic
//! acceptance) leaves the grant as it is and records the change here. The
//! daemon applies a record through the journal (`ShareHost`
//! `drain_exec_follow_ups_locked`); the same journal step that stores the
//! grant ends the record, so a record never outlives the grant it changed
//! and an explicit later change of the grant supersedes it.
use serde::{Deserialize, Serialize};

use crate::share::access_grant::AccessGrantChange;
use crate::share::profiles::fingerprint_matches;
use crate::share::{DirectGrantState, ExecGrantTarget, ShareProfiles};

/// The change of commands, terminal or screen of one Direct grant that
/// waits for the Exec-grant journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecFollowUp {
    pub device_id: String,
    pub public_key: String,
    pub change: AccessGrantChange,
    /// Unix seconds when the change was chosen.
    pub since: i64,
}

/// The same for one admitted member of a room (`RoomPolicy::member_follow_ups`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberFollowUp {
    pub device_id: String,
    pub change: AccessGrantChange,
}

/// Which record a follow-up is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FollowUpKey {
    Direct {
        device_id: String,
        public_key: String,
    },
    RoomMember {
        profile_id: String,
        device_id: String,
    },
}

/// What the daemon does with one record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FollowUpWork {
    /// Apply `change` to the exact grant `target` through the journal.
    Apply {
        target: ExecGrantTarget,
        change: AccessGrantChange,
    },
    /// The record can never be applied (the reason); it is dropped and the
    /// grant keeps what it has.
    Obsolete(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NextFollowUp {
    pub key: FollowUpKey,
    pub work: FollowUpWork,
}

/// A grant or member whose Exec rights can change has all four pins: the
/// public key equals the node id and the fingerprint belongs to the key.
/// The journal refuses everything else.
pub(crate) fn pins_complete(public_key: &str, node_id: &str, fingerprint: &str) -> bool {
    public_key == node_id && fingerprint_matches(public_key, fingerprint)
}

impl ShareProfiles {
    /// Whether any change of commands, terminal or screen waits for the
    /// journal.
    pub fn exec_follow_ups_waiting(&self) -> bool {
        !self.exec_follow_ups.is_empty()
            || self
                .rooms
                .iter()
                .any(|room| !room.policy.member_follow_ups.is_empty())
    }

    /// `change` of the Direct grant `(device_id, public_key)` waits for the
    /// journal; it replaces an earlier record of the device.
    pub(crate) fn defer_direct_exec(
        &mut self,
        device_id: &str,
        public_key: &str,
        change: AccessGrantChange,
        now: i64,
    ) {
        self.exec_follow_ups
            .retain(|follow_up| follow_up.device_id != device_id);
        self.exec_follow_ups.push(ExecFollowUp {
            device_id: device_id.into(),
            public_key: public_key.into(),
            change,
            since: now,
        });
    }

    /// The first record that waits (Direct grants first), with what the
    /// daemon does with it.
    pub(crate) fn next_exec_follow_up(&self) -> Option<NextFollowUp> {
        if let Some(follow_up) = self.exec_follow_ups.first() {
            return Some(NextFollowUp {
                key: FollowUpKey::Direct {
                    device_id: follow_up.device_id.clone(),
                    public_key: follow_up.public_key.clone(),
                },
                work: self.direct_follow_up_work(follow_up),
            });
        }
        for room in &self.rooms {
            if let Some(follow_up) = room.policy.member_follow_ups.first() {
                return Some(NextFollowUp {
                    key: FollowUpKey::RoomMember {
                        profile_id: room.id.clone(),
                        device_id: follow_up.device_id.clone(),
                    },
                    work: self.member_follow_up_work(room, follow_up),
                });
            }
        }
        None
    }

    fn direct_follow_up_work(&self, follow_up: &ExecFollowUp) -> FollowUpWork {
        let Some(grant) = self.direct_grants.iter().find(|grant| {
            grant.device_id == follow_up.device_id && grant.public_key == follow_up.public_key
        }) else {
            return FollowUpWork::Obsolete("Die Direkt-Freigabe besteht nicht mehr".into());
        };
        if grant.state != DirectGrantState::Accepted {
            return FollowUpWork::Obsolete("Die Direkt-Freigabe ist nicht aktiv".into());
        }
        if !pins_complete(&grant.public_key, &grant.node_id, &grant.fingerprint) {
            return FollowUpWork::Obsolete(
                "Die Direkt-Freigabe hat keine vollstaendig gepinnte Identitaet".into(),
            );
        }
        FollowUpWork::Apply {
            target: ExecGrantTarget::Direct {
                device_id: grant.device_id.clone(),
                public_key: grant.public_key.clone(),
                fingerprint: grant.fingerprint.clone(),
                node_id: grant.node_id.clone(),
            },
            change: follow_up.change.clone(),
        }
    }

    fn member_follow_up_work(
        &self,
        room: &crate::share::RoomProfile,
        follow_up: &MemberFollowUp,
    ) -> FollowUpWork {
        let Some(member) = room
            .members
            .iter()
            .find(|member| member.device_id == follow_up.device_id)
        else {
            return FollowUpWork::Obsolete("Das Raummitglied besteht nicht mehr".into());
        };
        if !room.auto_join || !member.is_admitted() {
            return FollowUpWork::Obsolete("Das Raummitglied ist nicht zugelassen".into());
        }
        if !pins_complete(&member.public_key, &member.node_id, &member.fingerprint) {
            return FollowUpWork::Obsolete(
                "Das Raummitglied hat keine vollstaendig gepinnte Identitaet".into(),
            );
        }
        FollowUpWork::Apply {
            target: ExecGrantTarget::RoomMember {
                room_id: room.room_id.clone(),
                device_id: member.device_id.clone(),
                public_key: member.public_key.clone(),
                fingerprint: member.fingerprint.clone(),
                node_id: member.node_id.clone(),
            },
            change: follow_up.change.clone(),
        }
    }

    /// Drops one record without applying it.
    pub(crate) fn discard_exec_follow_up(&mut self, key: &FollowUpKey) {
        match key {
            FollowUpKey::Direct {
                device_id,
                public_key,
            } => {
                self.exec_follow_ups.retain(|follow_up| {
                    !(follow_up.device_id == *device_id && follow_up.public_key == *public_key)
                });
                self.pending_rights
                    .retain(|pending| !pending.target.is_direct(device_id));
            }
            FollowUpKey::RoomMember {
                profile_id,
                device_id,
            } => {
                if let Some(room) = self.rooms.iter_mut().find(|room| &room.id == profile_id) {
                    room.policy
                        .member_follow_ups
                        .retain(|follow_up| &follow_up.device_id != device_id);
                }
            }
        }
    }

    /// The journal stored the grant `target`: a record that waited for it
    /// is done (or superseded by this explicit change), and with it the
    /// open confirmation the record kept open for a Direct device.
    pub(crate) fn finish_exec_follow_ups(&mut self, target: &ExecGrantTarget) {
        match target {
            ExecGrantTarget::Direct {
                device_id,
                public_key,
                ..
            } => {
                let before = self.exec_follow_ups.len();
                self.exec_follow_ups.retain(|follow_up| {
                    !(follow_up.device_id == *device_id && follow_up.public_key == *public_key)
                });
                if self.exec_follow_ups.len() != before {
                    self.pending_rights
                        .retain(|pending| !pending.target.is_direct(device_id));
                }
            }
            ExecGrantTarget::RoomMember {
                room_id, device_id, ..
            } => {
                if let Some(room) = self.rooms.iter_mut().find(|room| &room.room_id == room_id) {
                    room.policy
                        .member_follow_ups
                        .retain(|follow_up| &follow_up.device_id != device_id);
                }
            }
        }
    }

    /// Drops the records of grants that are gone or no longer active.
    pub(crate) fn prune_exec_follow_ups(&mut self) {
        let grants = &self.direct_grants;
        self.exec_follow_ups.retain(|follow_up| {
            grants.iter().any(|grant| {
                grant.device_id == follow_up.device_id
                    && grant.public_key == follow_up.public_key
                    && grant.state == DirectGrantState::Accepted
            })
        });
        for room in &mut self.rooms {
            let members = &room.members;
            room.policy.member_follow_ups.retain(|follow_up| {
                members
                    .iter()
                    .any(|member| member.device_id == follow_up.device_id)
            });
        }
    }
}

#[cfg(test)]
#[path = "abgleich_task_exec_follow_up_rooms_tests.rs"]
mod abgleich_task_room_tests;
#[cfg(test)]
#[path = "abgleich_task_exec_follow_up_tests.rs"]
mod abgleich_task_tests;
