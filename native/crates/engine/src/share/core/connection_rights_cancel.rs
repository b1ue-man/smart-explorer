//! „Abbrechen“ on the stored profiles (W2, review 3 of 2026-10-31): a
//! connection whose rights are open has no right afterwards, also when its
//! folders and write right were stored already and only commands, terminal
//! and screen wait for the Exec-grant journal (`ExecWrite::Deferred`). The
//! connection itself stays.
//!
//! Folders and write are taken back in the profile transaction. Commands,
//! terminal and screen that the grant holds now go away only through the
//! journal (`cancel_needs_journal`: the caller confirms
//! `rights_after_cancel` through the worker, which revokes at the running
//! worker before it stores). When that is not possible the profile
//! transaction records the revocation for the journal and the connection
//! stays „Rechte offen“ until the journal stored it; the worker holds the
//! grant masked meanwhile (`exec_follow_up.rs`).
use crate::share::access_grant::CapabilityDecision;
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, PendingRights, RightsOrigin, RightsTarget, MENU_CAPABILITIES,
};
use crate::share::ShareProfiles;

impl ShareProfiles {
    /// The rights `target` has after „Abbrechen“: what it holds now, with no
    /// folder, no write right and every command, terminal and screen
    /// decision off (account and end stay, as for every disabled grant).
    pub fn rights_after_cancel(&self, target: &RightsTarget) -> ConnectionRights {
        let current = match target {
            RightsTarget::Direct {
                device_id,
                public_key,
            } => self.current_direct_rights(device_id, public_key),
            RightsTarget::Room { profile_id } => self.current_room_rights(profile_id),
        };
        let mut rights = current.unwrap_or_else(ConnectionRights::none);
        rights.folders = FolderChoice::None;
        rights.write = false;
        for capability in MENU_CAPABILITIES {
            rights.set_decision(capability, CapabilityDecision::Off);
        }
        rights
    }

    /// Whether „Abbrechen“ of `target` has to go through the Exec-grant
    /// journal: a Direct device whose change of commands, terminal or
    /// screen waits for it, or whose open connection holds some of them
    /// now. A room and every other connection need only a profile
    /// transaction.
    pub fn cancel_needs_journal(&self, target: &RightsTarget) -> bool {
        let RightsTarget::Direct {
            device_id,
            public_key,
        } = target
        else {
            return false;
        };
        let waiting = self
            .exec_follow_ups
            .iter()
            .any(|follow_up| &follow_up.device_id == device_id);
        let holds_exec = self.pending_rights_of(target).is_some()
            && self.direct_grants.iter().any(|grant| {
                &grant.device_id == device_id
                    && &grant.public_key == public_key
                    && grant.exec.grants_any()
            });
        waiting || holds_exec
    }

    /// „Abbrechen“: the connection stays, without the rights it waited for.
    /// Returns whether anything was open. For a Direct device with open
    /// rights (a pending confirmation or a waiting change) folders and
    /// write right are taken back and the waiting change is dropped;
    /// commands, terminal and screen the grant holds now stay with the
    /// grant, and their revocation is recorded for the journal and keeps the
    /// connection open. A room only loses its open confirmation and the
    /// waiting changes of its members.
    pub fn cancel_connection_rights(&mut self, target: &RightsTarget, now: i64) -> bool {
        match target {
            RightsTarget::Direct {
                device_id,
                public_key,
            } => self.cancel_direct_rights(target, device_id, public_key, now),
            RightsTarget::Room { profile_id } => {
                let before = self.pending_rights.len();
                self.pending_rights
                    .retain(|pending| &pending.target != target);
                let mut dropped = false;
                if let Some(room) = self.rooms.iter_mut().find(|room| &room.id == profile_id) {
                    dropped = !room.policy.member_follow_ups.is_empty();
                    room.policy.member_follow_ups.clear();
                }
                dropped || before != self.pending_rights.len()
            }
        }
    }

    fn cancel_direct_rights(
        &mut self,
        target: &RightsTarget,
        device_id: &str,
        public_key: &str,
        now: i64,
    ) -> bool {
        let pending = self.pending_rights_of(target).cloned();
        let had_follow_up = self
            .exec_follow_ups
            .iter()
            .any(|follow_up| follow_up.device_id == device_id);
        if pending.is_none() && !had_follow_up {
            return false;
        }
        let revoke = self.rights_after_cancel(target).access_change();
        let mut exec_on = false;
        if let Some(grant) = self
            .direct_grants
            .iter_mut()
            .find(|grant| grant.device_id == device_id && grant.public_key == public_key)
        {
            exec_on = grant.exec.grants_any();
            let mut changed = std::mem::take(&mut grant.write);
            // `None` names no folder and cannot fail.
            changed |= self
                .default_direct_exports
                .set_device_folders(device_id, public_key, &FolderChoice::None)
                .unwrap_or(false);
            if changed {
                grant.updated_at = now;
            }
        }
        self.exec_follow_ups
            .retain(|follow_up| follow_up.device_id != device_id);
        self.pending_rights
            .retain(|pending| !pending.target.is_direct(device_id));
        if exec_on {
            self.defer_direct_exec(device_id, public_key, revoke, now);
            self.pending_rights.push(PendingRights {
                target: target.clone(),
                origin: pending
                    .as_ref()
                    .map_or(RightsOrigin::Request, |pending| pending.origin),
                since: pending.as_ref().map_or(now, |pending| pending.since),
            });
        }
        true
    }
}
