//! F1–F6 (plan 2026-10-31) on the stored profiles: a new connection starts
//! without any right and waits („Rechte offen“); confirming sets the chosen
//! rights in one step; cancelling keeps the connection without rights
//! (`connection_rights_cancel.rs`: also folders and write right that a
//! deferred confirmation stored already). Rights chosen in advance (Direct
//! code, PIN offer, room) are that confirmation. Existing grants and rooms
//! keep the rights they have.
use serde::{Deserialize, Serialize};

#[path = "connection_rights_cancel.rs"]
mod cancel;
#[path = "connection_rights_rooms.rs"]
mod rooms;

use crate::share::access_grant::AccessGrantChange;
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, PendingRights, RightsOrigin, RightsTarget, MENU_CAPABILITIES,
};
use crate::share::exec_policy::ExecGrant;
use crate::share::{DirectGrantState, ShareProfiles};

/// One open confirmation with what the menu needs to show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRightsView {
    pub target: RightsTarget,
    pub origin: RightsOrigin,
    pub since: i64,
    /// Device or room name.
    pub name: String,
    /// The rights the connection has now: none while it waits plain; when
    /// rights were chosen in advance (`ExecWrite::Deferred`) their folders
    /// and write right are in force already and only commands, terminal and
    /// screen wait for the Exec-grant journal („Abbrechen“ takes all of
    /// them back).
    pub rights: ConnectionRights,
}

/// Applies `change` to `exec` when it changes the access right; a refused
/// change leaves `exec` as it is.
fn apply_exec(exec: &mut ExecGrant, change: &AccessGrantChange, now: i64) -> Result<bool, String> {
    let mut next = exec.clone();
    next.apply_access_change(change, now)?;
    if next.access_grant() == exec.access_grant() && next.programs == exec.programs {
        return Ok(false);
    }
    *exec = next;
    Ok(true)
}

fn rights_from_access(rights: &mut ConnectionRights, exec: &ExecGrant) {
    let grant = exec.access_grant();
    for capability in MENU_CAPABILITIES {
        rights.set_decision(capability, grant.decision(capability));
    }
    rights.valid_until = grant.valid_until;
    rights.account = grant.account;
}

/// Who stores the commands, terminal and screen of a grant when rights are
/// applied. Only the Exec-grant journal of the background worker may set
/// them (`Journal`: the confirmation inside a journal entry, W8); every
/// other profile transaction leaves the grant as it is and records the
/// change for the journal (`Deferred`, `exec_follow_up.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecWrite {
    Journal,
    /// The origin a connection kept open for the deferred rights has.
    Deferred(RightsOrigin),
}

impl ShareProfiles {
    pub fn pending_rights_of(&self, target: &RightsTarget) -> Option<&PendingRights> {
        self.pending_rights
            .iter()
            .find(|pending| &pending.target == target)
    }

    /// Every open confirmation with its name and current rights.
    pub fn pending_rights_views(&self) -> Vec<PendingRightsView> {
        self.pending_rights
            .iter()
            .filter_map(|pending| {
                let (name, rights) = match &pending.target {
                    RightsTarget::Direct {
                        device_id,
                        public_key,
                    } => (
                        self.grant_for(device_id)?.device_name.clone(),
                        self.current_direct_rights(device_id, public_key)?,
                    ),
                    RightsTarget::Room { profile_id } => (
                        self.rooms
                            .iter()
                            .find(|room| &room.id == profile_id)?
                            .name
                            .clone(),
                        self.current_room_rights(profile_id)?,
                    ),
                };
                Some(PendingRightsView {
                    target: pending.target.clone(),
                    origin: pending.origin,
                    since: pending.since,
                    name,
                    rights,
                })
            })
            .collect()
    }

    /// F1: the grant of `device_id` was just created, or now belongs to a new
    /// key, by a new connection. It has no right at all; an accepted grant
    /// takes `advance` (rights chosen in advance: PIN offer) or the rights
    /// the user set for new connections („Direkt-Code vergibt“): for an
    /// accepted request (also the new request of a revoked or ignored
    /// device, `Readmitted`, W6) and, since R2 (plan 2026-11-06), for a PIN
    /// pairing, „Auch meine Freigaben öffnen“ and the background repair too.
    /// A new key of a known device (`IdentityReplaced`) never takes them.
    /// Without such rights it waits for confirmation. Rights chosen in
    /// advance that no longer apply (a folder is no longer exported) leave
    /// it waiting without rights.
    pub fn settle_new_direct_grant(
        &mut self,
        device_id: &str,
        origin: RightsOrigin,
        advance: Option<&ConnectionRights>,
        now: i64,
    ) {
        let Some(grant) = self
            .direct_grants
            .iter_mut()
            .find(|grant| grant.device_id == device_id)
        else {
            return;
        };
        grant.write = false;
        if grant.exec.grants_any() {
            grant.exec.disable_without_decision(now);
        }
        let public_key = grant.public_key.clone();
        let accepted = grant.state == DirectGrantState::Accepted;
        // `None` names no folder and cannot fail.
        let _ = self.default_direct_exports.set_device_folders(
            device_id,
            &public_key,
            &FolderChoice::None,
        );
        self.pending_rights
            .retain(|pending| !pending.target.is_direct(device_id));
        if !accepted {
            return;
        }
        let advance = advance.cloned().or_else(|| {
            matches!(
                origin,
                RightsOrigin::Request
                    | RightsOrigin::Readmitted
                    | RightsOrigin::Pairing
                    | RightsOrigin::ShareBack
                    | RightsOrigin::Repair
            )
            .then(|| self.code_rights.clone())
            .flatten()
        });
        if let Some(rights) = advance {
            // Commands, terminal and screen of rights chosen in advance
            // reach the grant only through the Exec-grant journal.
            if self
                .apply_direct_rights(
                    device_id,
                    &public_key,
                    &rights,
                    now,
                    ExecWrite::Deferred(origin),
                )
                .is_ok()
            {
                return;
            }
        }
        self.pending_rights.push(PendingRights {
            target: RightsTarget::Direct {
                device_id: device_id.into(),
                public_key,
            },
            origin,
            since: now,
        });
    }

    /// An accepted grant that allows nothing at all and is not waiting yet
    /// waits for confirmation (a deliberately readmitted grant).
    pub fn mark_rights_pending_if_rightless(
        &mut self,
        device_id: &str,
        origin: RightsOrigin,
        now: i64,
    ) {
        let Some(grant) = self.grant_for(device_id) else {
            return;
        };
        let target = RightsTarget::Direct {
            device_id: device_id.into(),
            public_key: grant.public_key.clone(),
        };
        let rightless = grant.state == DirectGrantState::Accepted
            && !grant.write
            && !grant.exec.grants_any()
            && self
                .default_direct_exports
                .folder_choice_of(device_id, &grant.public_key)
                == FolderChoice::None;
        if rightless && self.pending_rights_of(&target).is_none() {
            self.pending_rights.push(PendingRights {
                target,
                origin,
                since: now,
            });
        }
    }

    /// The rights the Direct grant `(device_id, public_key)` has now.
    pub fn current_direct_rights(
        &self,
        device_id: &str,
        public_key: &str,
    ) -> Option<ConnectionRights> {
        let grant = self
            .grant_for(device_id)
            .filter(|grant| grant.public_key == public_key)?;
        let mut rights = ConnectionRights {
            folders: self
                .default_direct_exports
                .folder_choice_of(device_id, public_key),
            write: grant.write,
            ..ConnectionRights::none()
        };
        rights_from_access(&mut rights, &grant.exec);
        Some(rights)
    }

    /// Sets exactly `rights` on the accepted Direct grant of `(device_id,
    /// public_key)` and ends its open confirmation, all or nothing. Returns
    /// whether anything changed. This is the step of the Exec-grant journal
    /// (`plan_confirm`): it stores commands, terminal and screen itself.
    /// Every other caller uses `apply_direct_connection_rights_deferring_exec`.
    pub fn apply_direct_connection_rights(
        &mut self,
        device_id: &str,
        public_key: &str,
        rights: &ConnectionRights,
        now: i64,
    ) -> Result<bool, String> {
        self.apply_direct_rights(device_id, public_key, rights, now, ExecWrite::Journal)
    }

    /// `apply_direct_connection_rights` for every profile transaction
    /// outside the journal (accepting a request with rights, the Direct
    /// code's rights, a PIN offer's rights): folders and write are set; a
    /// change of commands, terminal or screen leaves the grant as it is and
    /// is recorded for the journal (`exec_follow_ups`), and the connection
    /// stays open („Rechte offen“, with `origin`) until the journal stored
    /// it.
    pub fn apply_direct_connection_rights_deferring_exec(
        &mut self,
        device_id: &str,
        public_key: &str,
        rights: &ConnectionRights,
        origin: RightsOrigin,
        now: i64,
    ) -> Result<bool, String> {
        self.apply_direct_rights(
            device_id,
            public_key,
            rights,
            now,
            ExecWrite::Deferred(origin),
        )
    }

    fn apply_direct_rights(
        &mut self,
        device_id: &str,
        public_key: &str,
        rights: &ConnectionRights,
        now: i64,
        mode: ExecWrite,
    ) -> Result<bool, String> {
        rights.validate()?;
        let index = self
            .direct_grants
            .iter()
            .position(|grant| grant.device_id == device_id)
            .ok_or_else(|| format!("Keine Direkt-Freigabe fuer Geraet {device_id}"))?;
        let grant = &self.direct_grants[index];
        if grant.public_key != public_key {
            return Err(
                "Die Identitaet der Direkt-Freigabe wurde geaendert; bitte neu laden".into(),
            );
        }
        if rights.grants_anything() && grant.state != DirectGrantState::Accepted {
            return Err(
                "Rechte setzen eine aktive Freigabe voraus; zuerst ausdruecklich wieder erlauben"
                    .into(),
            );
        }
        let identity = crate::share::DirectPeerIdentity {
            device_id: grant.device_id.clone(),
            device_name: grant.device_name.clone(),
            public_key: grant.public_key.clone(),
            node_id: grant.node_id.clone(),
            fingerprint: grant.fingerprint.clone(),
        };
        if rights.grants_anything() && self.removed_direct_peer(&identity).is_some() {
            return Err("Das Geraet wurde entfernt; zuerst erneut koppeln oder zulassen".into());
        }
        let mut exports = self.default_direct_exports.clone();
        let mut changed = exports.set_device_folders(device_id, public_key, &rights.folders)?;
        let mut exec = grant.exec.clone();
        let exec_changed = apply_exec(&mut exec, &rights.access_change(), now)?;
        let deferred = exec_changed && matches!(mode, ExecWrite::Deferred(_));
        changed |= exec_changed;
        changed |= grant.write != rights.write;
        let grant = &mut self.direct_grants[index];
        grant.write = rights.write;
        if !deferred {
            grant.exec = exec;
        }
        if changed {
            grant.updated_at = now;
        }
        self.default_direct_exports = exports;
        let kept = self
            .pending_rights
            .iter()
            .find(|pending| pending.target.is_direct(device_id))
            .cloned();
        let before = self.pending_rights.len();
        let had_follow_up = self
            .exec_follow_ups
            .iter()
            .any(|follow_up| follow_up.device_id == device_id);
        self.pending_rights
            .retain(|pending| !pending.target.is_direct(device_id));
        self.exec_follow_ups
            .retain(|follow_up| follow_up.device_id != device_id);
        if let (true, ExecWrite::Deferred(origin)) = (deferred, mode) {
            self.defer_direct_exec(device_id, public_key, rights.access_change(), now);
            self.pending_rights.push(PendingRights {
                target: RightsTarget::Direct {
                    device_id: device_id.into(),
                    public_key: public_key.into(),
                },
                origin: kept.as_ref().map_or(origin, |pending| pending.origin),
                since: kept.as_ref().map_or(now, |pending| pending.since),
            });
            return Ok(true);
        }
        Ok(changed || before != self.pending_rights.len() || had_follow_up)
    }

    /// „Bestätigen“: sets exactly `rights` on `target` (it need not wait).
    pub fn confirm_connection_rights(
        &mut self,
        target: &RightsTarget,
        rights: &ConnectionRights,
        now: i64,
    ) -> Result<bool, String> {
        match target {
            RightsTarget::Direct {
                device_id,
                public_key,
            } => self.apply_direct_connection_rights(device_id, public_key, rights, now),
            RightsTarget::Room { profile_id } => {
                self.apply_room_connection_rights(profile_id, rights, now)
            }
        }
    }

    /// F5/F6: the rights for devices that connect with this Direct code;
    /// `None` lets them wait without rights. Returns whether it changed.
    pub fn set_code_rights(&mut self, rights: Option<ConnectionRights>) -> Result<bool, String> {
        if let Some(rights) = &rights {
            rights.validate()?;
        }
        let changed = self.code_rights != rights;
        self.code_rights = rights;
        Ok(changed)
    }

    /// Drops open confirmations without their accepted grant or room, and
    /// folder selections of devices without a grant.
    pub fn prune_connection_rights_records(&mut self) {
        let grants = &self.direct_grants;
        let rooms = &self.rooms;
        self.pending_rights.retain(|pending| match &pending.target {
            RightsTarget::Direct {
                device_id,
                public_key,
            } => grants.iter().any(|grant| {
                &grant.device_id == device_id
                    && &grant.public_key == public_key
                    && grant.state == DirectGrantState::Accepted
            }),
            RightsTarget::Room { profile_id } => rooms.iter().any(|room| &room.id == profile_id),
        });
        self.default_direct_exports
            .retain_device_folders(|entry| grants.iter().any(|g| g.device_id == entry.device_id));
        self.prune_exec_follow_ups();
    }
}
