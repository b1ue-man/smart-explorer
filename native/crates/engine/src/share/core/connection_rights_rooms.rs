//! F1/F5 for rooms (plan 2026-10-31): a room created or joined here waits
//! for its rights; confirming sets its exports, the members' write right
//! and the access every admitted member gets, now and when admitted later.
use super::{apply_exec, ExecWrite};
use crate::share::access_grant::{AccessGrantChange, Capability, Expiry};
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, PendingRights, RightsOrigin, RightsTarget, MENU_CAPABILITIES,
};
use crate::share::exec_follow_up::MemberFollowUp;
use crate::share::{ExportAccess, RoomProfile, ShareExportConfig, ShareProfiles, SharedRoot};

/// The access change a room keeps for its later members: empty when the
/// rights allow no command, terminal or screen.
fn member_template(rights: &ConnectionRights) -> AccessGrantChange {
    if MENU_CAPABILITIES
        .into_iter()
        .any(|capability| rights.decision(capability).is_on())
    {
        rights.access_change()
    } else {
        AccessGrantChange::default()
    }
}

fn room_root(root: &SharedRoot, write: bool) -> SharedRoot {
    let mut root = root.clone();
    if !write {
        root.access = ExportAccess::ReadOnly;
        root.allow_system_writes = false;
    }
    root
}

/// The exports of a room for `rights`: the Direct folders (`All`), none, or
/// the selected ones, taken from the room itself or the Direct folders.
fn room_exports(
    direct: &ShareExportConfig,
    current: &ShareExportConfig,
    rights: &ConnectionRights,
) -> Result<ShareExportConfig, String> {
    let connections = |source: &ShareExportConfig| {
        source
            .shared_connections
            .iter()
            .cloned()
            .map(|mut connection| {
                if !rights.write {
                    connection.access = ExportAccess::ReadOnly;
                }
                connection
            })
            .collect::<Vec<_>>()
    };
    Ok(match &rights.folders {
        FolderChoice::None => ShareExportConfig::default(),
        FolderChoice::All => ShareExportConfig {
            roots: direct
                .roots
                .iter()
                .map(|root| room_root(root, rights.write))
                .collect(),
            shared_connections: connections(direct),
            ..ShareExportConfig::default()
        },
        FolderChoice::Selected {
            paths,
            connections: with_connections,
        } => {
            let mut roots = Vec::new();
            for path in paths {
                let root = current
                    .roots
                    .iter()
                    .chain(&direct.roots)
                    .find(|root| &root.path == path)
                    .ok_or_else(|| {
                        format!("Ordner ist weder Raum- noch Direkt-Freigabe: {path}")
                    })?;
                roots.push(room_root(root, rights.write));
            }
            let source = if current.shared_connections.is_empty() {
                direct
            } else {
                current
            };
            ShareExportConfig {
                roots,
                shared_connections: if *with_connections {
                    connections(source)
                } else {
                    Vec::new()
                },
                ..ShareExportConfig::default()
            }
        }
    })
}

impl ShareProfiles {
    /// F1 for a room created or joined here: it waits for its rights.
    pub fn mark_room_rights_pending(
        &mut self,
        profile_id: &str,
        origin: RightsOrigin,
        now: i64,
    ) {
        let target = RightsTarget::Room {
            profile_id: profile_id.into(),
        };
        if self.pending_rights_of(&target).is_none() {
            self.pending_rights.push(PendingRights {
                target,
                origin,
                since: now,
            });
        }
    }

    /// The rights of the room `profile_id`: its folders, the members' write
    /// right and the access kept for its members.
    pub fn current_room_rights(&self, profile_id: &str) -> Option<ConnectionRights> {
        let room = self.rooms.iter().find(|room| room.id == profile_id)?;
        let connections = room.exports.shares_connections();
        let mut rights = ConnectionRights {
            folders: if room.exports.roots.is_empty() && !connections {
                FolderChoice::None
            } else {
                FolderChoice::Selected {
                    paths: room
                        .exports
                        .roots
                        .iter()
                        .map(|root| root.path.clone())
                        .collect(),
                    connections,
                }
            },
            write: room.policy.members_may_write,
            ..ConnectionRights::none()
        };
        let template = &room.policy.member_access;
        for capability in MENU_CAPABILITIES {
            if let Some(decision) = template.capabilities.get(&capability) {
                rights.set_decision(capability, *decision);
            }
        }
        if let Some(Expiry::At(end)) = template.expiry {
            rights.valid_until = Some(end);
        }
        if let Some(account) = &template.account {
            rights.account = account.clone();
        }
        Some(rights)
    }

    /// Sets exactly `rights` on the room `profile_id`: its exports (the
    /// Direct folders for `All`), the members' write right, and commands,
    /// terminal and screen for every admitted member now and every member
    /// admitted later. All or nothing; ends the room's open confirmation.
    /// This is the step of the Exec-grant journal (`plan_confirm`): it
    /// stores the members' commands, terminal and screen itself. Every
    /// other caller uses `apply_room_connection_rights_deferring_exec`.
    pub fn apply_room_connection_rights(
        &mut self,
        profile_id: &str,
        rights: &ConnectionRights,
        now: i64,
    ) -> Result<bool, String> {
        self.apply_room_rights(profile_id, rights, now, ExecWrite::Journal)
    }

    /// `apply_room_connection_rights` for every profile transaction outside
    /// the journal (the rights chosen when a room is created): exports, write
    /// and the access kept for later members are set; a change of commands,
    /// terminal or screen of a member admitted now leaves its grant as it is
    /// and is recorded for the journal (`RoomPolicy::member_follow_ups`).
    pub fn apply_room_connection_rights_deferring_exec(
        &mut self,
        profile_id: &str,
        rights: &ConnectionRights,
        now: i64,
    ) -> Result<bool, String> {
        self.apply_room_rights(
            profile_id,
            rights,
            now,
            ExecWrite::Deferred(RightsOrigin::RoomCreated),
        )
    }

    fn apply_room_rights(
        &mut self,
        profile_id: &str,
        rights: &ConnectionRights,
        now: i64,
        mode: ExecWrite,
    ) -> Result<bool, String> {
        rights.validate()?;
        let index = self
            .rooms
            .iter()
            .position(|room| room.id == profile_id)
            .ok_or_else(|| format!("Raumprofil nicht gefunden: {profile_id}"))?;
        let mut room = self.rooms[index].clone();
        room.exports = room_exports(&self.default_direct_exports, &room.exports, rights)?;
        room.policy.members_may_write = rights.write;
        room.policy.member_access = member_template(rights);
        let change = rights.access_change();
        let mut waiting = Vec::new();
        for member in room
            .members
            .iter_mut()
            .filter(|member| member.is_admitted())
        {
            match mode {
                ExecWrite::Journal => {
                    apply_exec(&mut member.exec, &change, now)?;
                }
                ExecWrite::Deferred(_) => {
                    let mut next = member.exec.clone();
                    if apply_exec(&mut next, &change, now)? {
                        waiting.push(MemberFollowUp {
                            device_id: member.device_id.clone(),
                            change: change.clone(),
                        });
                    }
                }
            }
        }
        // The rights chosen now replace every earlier record of the room.
        room.policy.member_follow_ups = waiting;
        let mut changed = room != self.rooms[index];
        self.rooms[index] = room;
        let before = self.pending_rights.len();
        self.pending_rights
            .retain(|pending| !pending.target.is_room(profile_id));
        changed |= before != self.pending_rights.len();
        Ok(changed)
    }
}

impl RoomProfile {
    /// F5: the room's member access for the admitted member `device_id`
    /// that allows nothing yet. The member's grant is not changed here: the
    /// access is recorded for the Exec-grant journal
    /// (`RoomPolicy::member_follow_ups`), which stores it. Returns whether a
    /// record was made.
    pub fn apply_member_access(&mut self, device_id: &str, now: i64) -> bool {
        if self.policy.member_access.is_empty() {
            return false;
        }
        let change = self.policy.member_access.clone();
        let Some(member) = self
            .members
            .iter()
            .find(|member| member.device_id == device_id && member.is_admitted())
        else {
            return false;
        };
        if member.exec.grants_any()
            || change
                .capabilities
                .get(&Capability::LoginAccess)
                .is_some_and(|decision| decision.is_on())
        {
            return false;
        }
        let mut next = member.exec.clone();
        if !apply_exec(&mut next, &change, now).unwrap_or(false) {
            return false;
        }
        let follow_up = MemberFollowUp {
            device_id: device_id.to_string(),
            change,
        };
        if self.policy.member_follow_ups.contains(&follow_up) {
            return false;
        }
        self.policy
            .member_follow_ups
            .retain(|waiting| waiting.device_id != device_id);
        self.policy.member_follow_ups.push(follow_up);
        true
    }
}
