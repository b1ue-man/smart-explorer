//! User settings of contacts, rooms and export order that every surface (CLI,
//! desktop, Android) changes through the same profile transaction
//! (`ShareProfiles::mutate_persisted`). Each edit names its object by the
//! stable local id and returns whether anything changed.
use super::super::room_relation::canonical_room_display_name;
use crate::share::{ShareProfiles, ShareStatus};

impl ShareProfiles {
    /// „Automatisch verbinden“ / „Automatisch öffnen“ of one direct contact;
    /// `None` keeps a value.
    pub fn set_contact_options(
        &mut self,
        contact_id: &str,
        auto_connect: Option<bool>,
        auto_open: Option<bool>,
    ) -> Result<bool, String> {
        let contact = self
            .direct_contacts
            .iter_mut()
            .find(|contact| contact.id == contact_id)
            .ok_or_else(|| format!("Direktgeraet nicht gefunden: {contact_id}"))?;
        let before = (contact.auto_connect, contact.auto_open);
        if let Some(value) = auto_connect {
            contact.auto_connect = value;
        }
        if let Some(value) = auto_open {
            contact.auto_open = value;
        }
        Ok(before != (contact.auto_connect, contact.auto_open))
    }

    /// „Vertrauen zurücksetzen“ of one direct contact: the pinned remote
    /// identity is forgotten and its grant withdrawn (Exec off), so the
    /// device must be confirmed again; the contact itself stays.
    pub fn reset_contact_trust(&mut self, contact_id: &str, now: i64) -> Result<bool, String> {
        let contact = self
            .direct_contacts
            .iter()
            .find(|contact| contact.id == contact_id)
            .ok_or_else(|| format!("Direktgeraet nicht gefunden: {contact_id}"))?;
        let pinned = Self::contact_remote_identity(contact);
        let changed = pinned.is_some()
            || contact.remote_public_key.is_some()
            || contact.presence.is_some()
            || contact.status != ShareStatus::Waiting;
        if let Some(peer) = &pinned {
            self.withdraw_direct_key(peer, now);
        }
        if let Some(contact) = self
            .direct_contacts
            .iter_mut()
            .find(|contact| contact.id == contact_id)
        {
            contact.remote_device_id = None;
            contact.remote_public_key = None;
            contact.presence = None;
            contact.status = ShareStatus::Waiting;
        }
        Ok(changed)
    }

    /// Local display name of a room (the rules of joining: trimmed, empty =
    /// „Raum“, at most 256 bytes, no control characters).
    pub fn rename_room(&mut self, profile_id: &str, name: &str) -> Result<bool, String> {
        let name = canonical_room_display_name(name).map_err(|error| error.to_string())?;
        let room = self
            .rooms
            .iter_mut()
            .find(|room| room.id == profile_id)
            .ok_or_else(|| format!("Raumprofil nicht gefunden: {profile_id}"))?;
        let changed = room.name != name;
        room.name = name;
        Ok(changed)
    }

    /// „Beitreten“ (`true`) or „Verlassen“ (`false`) of a saved room; leaving
    /// also marks it offline until the worker's next snapshot.
    pub fn set_room_auto_join(
        &mut self,
        profile_id: &str,
        auto_join: bool,
    ) -> Result<bool, String> {
        let room = self
            .rooms
            .iter_mut()
            .find(|room| room.id == profile_id)
            .ok_or_else(|| format!("Raumprofil nicht gefunden: {profile_id}"))?;
        let changed = room.auto_join != auto_join;
        room.auto_join = auto_join;
        if !auto_join {
            room.status = ShareStatus::Offline;
        }
        Ok(changed)
    }

    /// „Vertrauen zurücksetzen“ of a room member: its cached presence is
    /// dropped and looked up again; admission and Exec stay as they are.
    pub fn reset_member_trust(
        &mut self,
        profile_id: &str,
        device_id: &str,
    ) -> Result<bool, String> {
        let member = self
            .rooms
            .iter_mut()
            .find(|room| room.id == profile_id)
            .ok_or_else(|| format!("Raumprofil nicht gefunden: {profile_id}"))?
            .members
            .iter_mut()
            .find(|member| member.device_id == device_id)
            .ok_or_else(|| format!("Raummitglied nicht gefunden: {device_id}"))?;
        let changed = member.presence.is_some() || member.status != ShareStatus::Waiting;
        member.presence = None;
        member.status = ShareStatus::Waiting;
        Ok(changed)
    }

    /// Moves the export `path` of `scope` (`direct` or a room profile id) by
    /// `offset` places; peers see the exports in this order. Returns `false`
    /// when it already stands at that end.
    pub fn move_export_root(
        &mut self,
        scope: &str,
        path: &str,
        offset: isize,
    ) -> Result<bool, String> {
        let config = self.export_config_mut(scope)?;
        let index = config
            .roots
            .iter()
            .position(|root| root.path == path)
            .ok_or_else(|| format!("Freigabe nicht gefunden: {path}"))?;
        let last = config.roots.len().saturating_sub(1) as isize;
        let target = (index as isize).saturating_add(offset).clamp(0, last) as usize;
        if target == index {
            return Ok(false);
        }
        let root = config.roots.remove(index);
        config.roots.insert(target, root);
        Ok(true)
    }
}

#[cfg(test)]
#[path = "profile_settings_tests.rs"]
mod luecken_task_tests;
