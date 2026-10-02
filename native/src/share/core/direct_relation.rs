//! Direct relation records and the rights they carry (contract V5).
//!
//! A [`DirectContact`] is this device's access *to* a peer (outgoing); a
//! [`DirectGrant`] is a peer's access *to this device* (incoming). Rights of a
//! grant (read, write, Exec) belong to one exact identity. Runtime data such as
//! presence, LAN routes or status never changes a right (FA3).
use serde::{Deserialize, Serialize};

use super::exec_policy::ExecGrant;
use super::profiles::ShareProfiles;
use super::types::{PeerPresence, ShareStatus};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DirectAccessState {
    Pending,
    Accepted,
    Ignored,
    IdentityConflict,
}

impl DirectAccessState {
    pub fn label(&self) -> &'static str {
        match self {
            DirectAccessState::Pending => "Warte auf Freigabe",
            DirectAccessState::Accepted => "Freigegeben",
            DirectAccessState::Ignored => "Ignoriert",
            DirectAccessState::IdentityConflict => "Identitaetskonflikt",
        }
    }
}

pub(crate) fn default_direct_access_state() -> DirectAccessState {
    DirectAccessState::Accepted
}

/// Grants and rooms persisted before V5 keep the write access they had (FC1:
/// existing accepted relations keep writing where the export allows it).
pub(crate) fn legacy_relation_write() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DirectGrantState {
    /// Access with the rights of the grant (`write`, `exec`).
    Accepted,
    /// Explicitly denied by the user („gesperrt“). Requests from this exact
    /// identity are rejected; only a deliberate act of the user („Wieder
    /// erlauben“) reactivates the grant, always with Exec off.
    Ignored,
    /// Suspended after this device's Direct-code rotation or identity repair
    /// („neu bestätigen“). Not a user denial: a request from the same identity
    /// authenticated with the current code, a deliberate pairing or the user
    /// reactivates it, always with Exec off. Automatic repair never does.
    Reconfirm,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectGrant {
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    #[serde(default)]
    pub node_id: String,
    pub state: DirectGrantState,
    pub updated_at: i64,
    #[serde(default)]
    pub exec: ExecGrant,
    /// FC1 „Darf schreiben“: the peer may write where an export is
    /// read-write. New grants start without it; grants persisted before V5
    /// keep it.
    #[serde(default = "legacy_relation_write")]
    pub write: bool,
}

impl DirectGrant {
    /// Whether this grant authorizes an incoming Direct session of exactly
    /// this identity: accepted, same device and key, and the pinned node (a
    /// grant without node pin accepts only `node_id == public_key`). The
    /// caller still checks the fingerprint and the session proof.
    pub(crate) fn authorizes_session(
        &self,
        device_id: &str,
        public_key: &str,
        node_id: &str,
    ) -> bool {
        self.state == DirectGrantState::Accepted
            && self.device_id == device_id
            && self.public_key == public_key
            && (self.node_id == node_id || (self.node_id.is_empty() && self.public_key == node_id))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectContact {
    pub id: String,
    pub display_name: String,
    pub lookup_id: String,
    pub expected_fingerprint: String,
    #[serde(default)]
    pub expected_node_id: String,
    pub remote_device_id: Option<String>,
    pub remote_public_key: Option<String>,
    pub auto_connect: bool,
    pub auto_open: bool,
    pub last_seen: Option<i64>,
    #[serde(default)]
    pub status: ShareStatus,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub presence: Option<PeerPresence>,
    #[serde(default = "default_direct_access_state")]
    pub access_state: DirectAccessState,
    #[serde(default)]
    pub request_sent_at: Option<i64>,
    #[serde(default)]
    pub accepted_at: Option<i64>,
    #[serde(default)]
    pub accepted_public_key: Option<String>,
    #[serde(default)]
    pub lan_candidates: Vec<String>,
    #[serde(default)]
    pub lan_seen_at: Option<i64>,
    #[serde(default)]
    pub lan_uplink: Option<bool>,
    #[serde(default)]
    pub relation: DirectRelationFlags,
}

/// Per-contact relation choices and learned security facts (V5).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectRelationFlags {
    /// FC1 „Auch meine Freigaben für dieses Gerät öffnen“: the reciprocal
    /// repair may create or reactivate a grant for this contact's device. Off
    /// for new contacts and for contacts persisted before V5; an existing
    /// grant is never removed by this flag.
    #[serde(default)]
    pub share_back: bool,
    /// B03: the peer delivered a presence signed with its pinned key; unsigned
    /// presences for this contact are rejected from then on.
    #[serde(default)]
    pub signed_presence: bool,
}

/// Whether incoming Direct requests from identities without a grant are
/// accepted automatically (FC5). The default waits for the user.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum DirectRequestPolicy {
    #[default]
    Ask,
    /// „Anfragen mit meinem Code automatisch annehmen (unsicherer)“.
    AutoAccept,
}

/// FA3: runtime half of one Direct contact, forwarded without a configuration
/// transition (`ShareCmd::UpdateRuntime`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectContactRuntime {
    pub contact_id: String,
    pub status: ShareStatus,
    pub last_seen: Option<i64>,
    pub last_error: Option<String>,
    pub presence: Option<PeerPresence>,
    pub lan_candidates: Vec<String>,
    pub lan_seen_at: Option<i64>,
    pub lan_uplink: Option<bool>,
}

impl DirectContactRuntime {
    pub(crate) fn of(contact: &DirectContact) -> Self {
        Self {
            contact_id: contact.id.clone(),
            status: contact.status.clone(),
            last_seen: contact.last_seen,
            last_error: contact.last_error.clone(),
            presence: contact.presence.clone(),
            lan_candidates: contact.lan_candidates.clone(),
            lan_seen_at: contact.lan_seen_at,
            lan_uplink: contact.lan_uplink,
        }
    }

    /// Copies only runtime fields; pins, access state and relation flags stay.
    /// Returns whether anything changed.
    pub(crate) fn apply_to(&self, contact: &mut DirectContact) -> bool {
        let before = Self::of(contact);
        if before == *self {
            return false;
        }
        contact.status = self.status.clone();
        contact.last_seen = self.last_seen;
        contact.last_error = self.last_error.clone();
        contact.presence = self.presence.clone();
        contact.lan_candidates = self.lan_candidates.clone();
        contact.lan_seen_at = self.lan_seen_at;
        contact.lan_uplink = self.lan_uplink;
        true
    }
}

/// Applies runtime updates to the contacts with the same id; unknown ids are
/// ignored (contacts are added only through the configuration path).
pub(crate) fn apply_contact_runtime(
    contacts: &mut [DirectContact],
    runtime: &[DirectContactRuntime],
) -> bool {
    let mut changed = false;
    for update in runtime {
        if let Some(contact) = contacts
            .iter_mut()
            .find(|contact| contact.id == update.contact_id)
        {
            changed |= update.apply_to(contact);
        }
    }
    changed
}

impl ShareProfiles {
    /// FC1 „Darf schreiben“ for the grant of `device_id`. Returns whether the
    /// value changed. Withdrawing write is a restriction (V5 invalidation).
    pub fn set_direct_grant_write(
        &mut self,
        device_id: &str,
        write: bool,
        now: i64,
    ) -> Result<bool, String> {
        let mut found = false;
        let mut changed = false;
        for grant in self
            .direct_grants
            .iter_mut()
            .filter(|grant| grant.device_id == device_id)
        {
            found = true;
            if grant.write != write {
                grant.write = write;
                grant.updated_at = now;
                changed = true;
            }
        }
        if !found {
            return Err(format!("Keine Direkt-Freigabe fuer Geraet {device_id}"));
        }
        Ok(changed)
    }
}
