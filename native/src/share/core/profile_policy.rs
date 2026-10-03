//! Explicit FC1 configuration edits. Endpoint/account strings are identifiers,
//! never normalized here. Editing rights neither pairs nor repairs a peer.
use super::direct_protocol::DirectPeerIdentity;
use super::export_config::{ExportAccess, ShareExportConfig, SharedConnection};
use super::profiles::ShareProfiles;
use super::types::DirectGrantState;

impl ShareProfiles {
    pub fn export_config_mut(&mut self, scope: &str) -> Result<&mut ShareExportConfig, String> {
        if scope == "direct" {
            return Ok(&mut self.default_direct_exports);
        }
        self.rooms
            .iter_mut()
            .find(|room| room.id == scope)
            .map(|room| &mut room.exports)
            .ok_or_else(|| format!("Raumprofil nicht gefunden: {scope}"))
    }

    /// Recheck every pin after reloading, including on a CAS retry. An inactive
    /// authorization must be explicitly readmitted before write can be enabled.
    pub fn set_direct_peer_write(
        &mut self,
        expected: &DirectPeerIdentity,
        write: bool,
        now: i64,
    ) -> Result<bool, String> {
        let matches = self
            .direct_grants
            .iter()
            .filter(|grant| grant.device_id == expected.device_id)
            .collect::<Vec<_>>();
        let [grant] = matches.as_slice() else {
            return Err("Direkt-Freigabe fehlt oder ist nicht eindeutig; bitte neu laden".into());
        };
        if grant.public_key != expected.public_key
            || grant.node_id != expected.node_id
            || grant.fingerprint != expected.fingerprint
        {
            return Err(
                "Die Identitaet der Direkt-Freigabe wurde geaendert; bitte neu laden".into(),
            );
        }
        if write
            && (grant.state != DirectGrantState::Accepted
                || self.removed_direct_peer(expected).is_some())
        {
            return Err(
                "Schreiben setzt eine aktive, ausdruecklich erlaubte Freigabe voraus".into(),
            );
        }
        self.set_direct_grant_write(&expected.device_id, write, now)
    }

    pub fn set_room_policy(
        &mut self,
        profile_id: &str,
        members_may_write: Option<bool>,
        confirm_new_members: Option<bool>,
    ) -> Result<bool, String> {
        let room = self
            .rooms
            .iter_mut()
            .find(|room| room.id == profile_id)
            .ok_or_else(|| format!("Raumprofil nicht gefunden: {profile_id}"))?;
        let before = room.policy.clone();
        if let Some(write) = members_may_write {
            room.policy.members_may_write = write;
        }
        if let Some(confirm) = confirm_new_members {
            room.policy.confirm_new_members = confirm;
        }
        // Existing pending/blocked admissions and their denial history survive
        // a policy change. Admitting a member is a separate explicit operation.
        Ok(room.policy != before)
    }
}

#[cfg(test)]
#[path = "profile_policy_tests.rs"]
mod tests;

impl ShareExportConfig {
    pub fn set_root_access(
        &mut self,
        path: &str,
        access: ExportAccess,
        allow_system_writes: Option<bool>,
    ) -> Result<bool, String> {
        let mut roots = self.roots.iter_mut().filter(|root| root.path == path);
        let root = roots
            .next()
            .ok_or_else(|| format!("Freigabe nicht gefunden: {path}"))?;
        if roots.next().is_some() {
            return Err("Freigabepfad ist nicht eindeutig".into());
        }
        let changed = root.access != access
            || allow_system_writes.is_some_and(|allow| root.allow_system_writes != allow);
        root.access = access;
        if let Some(allow) = allow_system_writes {
            root.allow_system_writes = allow;
        }
        Ok(changed)
    }

    /// `None` withdraws this account. A newly selected account starts read-only
    /// unless a writing choice was explicitly supplied by the user.
    pub fn set_connection_access(
        &mut self,
        account: &str,
        access: Option<ExportAccess>,
    ) -> Result<bool, String> {
        if account.is_empty() || account.chars().any(char::is_control) {
            return Err("Ungueltige Verbindungsidentitaet".into());
        }
        if self.include_connections {
            return Err(
                "Alte Verbindungsfreigaben muessen zuerst persistiert migriert werden".into(),
            );
        }
        let old = self.connection_access(account);
        match access {
            None => self
                .shared_connections
                .retain(|connection| connection.account != account),
            Some(access) => match self
                .shared_connections
                .iter_mut()
                .find(|connection| connection.account == account)
            {
                Some(connection) => connection.access = access,
                None => self.shared_connections.push(SharedConnection {
                    account: account.into(),
                    access,
                }),
            },
        }
        Ok(old != access)
    }
}
