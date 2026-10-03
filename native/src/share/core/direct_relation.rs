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
    /// Revocation belongs to the cryptographic principal, including every
    /// device-id alias of that key/node. A spoofed device-id claim alone never
    /// withdraws another key's authority.
    pub(crate) fn withdraw_direct_key(
        &mut self,
        peer: &super::direct_protocol::DirectPeerIdentity,
        now: i64,
    ) {
        for grant in &mut self.direct_grants {
            if (!peer.public_key.is_empty() && grant.public_key == peer.public_key
                || !peer.node_id.is_empty() && grant.node_id == peer.node_id)
                && (grant.state != DirectGrantState::Ignored || grant.exec.enabled)
            {
                grant.state = DirectGrantState::Ignored;
                grant.exec.disable_without_decision(now);
                grant.updated_at = now;
            }
        }
        self.mark_legacy_revoked_for_peer(peer, now);
    }

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

impl ShareProfiles {
    /// „Wieder erlauben“ (blocked) or „Bestätigen“ (suspended after a code
    /// rotation): the grant of `device_id` is active again, Exec stays off
    /// (B04) and earlier removal records of this key are lifted, because this
    /// is the user's own decision. Returns whether anything changed.
    pub fn allow_direct_grant_again(&mut self, device_id: &str, now: i64) -> Result<bool, String> {
        let grant = self.direct_grants.iter().find(|grant| grant.device_id == device_id)
            .ok_or_else(|| format!("Keine Direkt-Freigabe fuer Geraet {device_id}"))?;
        let identity = super::direct_protocol::DirectPeerIdentity {
            device_id: grant.device_id.clone(), device_name: grant.device_name.clone(),
            node_id: grant.node_id.clone(), public_key: grant.public_key.clone(),
            fingerprint: grant.fingerprint.clone(),
        };
        let matches = |grant: &DirectGrant| {
            !identity.public_key.is_empty() && grant.public_key == identity.public_key
                || !identity.node_id.is_empty() && grant.node_id == identity.node_id
        };
        if self.removed_direct_peer(&identity).is_none()
            && !self.direct_grants.iter().any(|grant| matches(grant)
                && grant.state != DirectGrantState::Accepted)
        {
            return Ok(false);
        }
        for alias in &mut self.direct_grants {
            if matches(alias) && (alias.state != DirectGrantState::Accepted || alias.exec.enabled) {
                alias.state = DirectGrantState::Accepted;
                alias.exec.disable_without_decision(now);
                alias.updated_at = now;
            }
        }
        self.readmit_removed_direct_identity(&identity);
        self.recompute_identity_conflicts_for_device(device_id);
        Ok(true)
    }

    /// FC1 „Auch meine Freigaben für dieses Gerät öffnen“ for a contact. With
    /// `share_back` on, an accepted grant of the contact's known device is
    /// created now (read-only unless „Darf schreiben“) and the reciprocal
    /// repair may create it later; off keeps an existing grant (revoking it is
    /// a separate action). Returns whether anything changed.
    pub fn set_contact_share_back(
        &mut self,
        contact_id: &str,
        share_back: bool,
        now: i64,
    ) -> Result<bool, String> {
        let contact = self
            .direct_contacts
            .iter_mut()
            .find(|contact| contact.id == contact_id)
            .ok_or_else(|| format!("Direktgeraet nicht gefunden: {contact_id}"))?;
        let mut changed = contact.relation.share_back != share_back;
        contact.relation.share_back = share_back;
        if !share_back {
            return Ok(changed);
        }
        let Some(identity) = Self::contact_remote_identity(contact) else {
            return Ok(changed);
        };
        if identity.public_key.is_empty() {
            return Ok(changed);
        }
        if self.removed_direct_peer(&identity).is_some() {
            self.readmit_removed_direct_identity(&identity);
            changed = true;
        }
        match self
            .direct_grants
            .iter_mut()
            .find(|grant| grant.device_id == identity.device_id)
        {
            Some(grant) if grant.public_key != identity.public_key => {
                return Err(format!(
                    "Direkt-Freigabe von {} gehoert zu einem anderen Schluessel",
                    identity.device_id
                ));
            }
            Some(grant) if grant.state == DirectGrantState::Accepted => {}
            Some(grant) => {
                grant.state = DirectGrantState::Accepted;
                if grant.exec.enabled {
                    grant.exec.disable_without_decision(now);
                }
                grant.updated_at = now;
                changed = true;
            }
            None => {
                self.direct_grants.push(DirectGrant {
                    device_id: identity.device_id.clone(),
                    device_name: identity.device_name.clone(),
                    public_key: identity.public_key.clone(),
                    fingerprint: identity.fingerprint.clone(),
                    node_id: identity.node_id.clone(),
                    state: DirectGrantState::Accepted,
                    updated_at: now,
                    exec: ExecGrant::default(),
                    write: false,
                });
                changed = true;
            }
        }
        // The user's choice re-admits this key, including a persisted alias
        // that an older profile kept blocked under another device id.
        changed |= self.allow_direct_grant_again(&identity.device_id, now)?;
        Ok(changed)
    }
}

/// What a verified presence did to a contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceApply {
    /// Only runtime data changed (presence, status, `last_seen`).
    Runtime,
    /// An empty pin was filled or the first valid signature remembered: the
    /// worker needs the new configuration.
    Pinned,
    /// The presence names another device, key or node than the pinned one;
    /// it is left out and the conflict is shown.
    Conflict,
}

impl DirectContact {
    /// Applies a presence the worker verified (DirectAvailable). Pinned values
    /// are never replaced (S29, S30); empty pins are filled once (TOFU) and a
    /// valid signature is remembered so unsigned presences fail from then on.
    pub fn apply_verified_presence(&mut self, presence: PeerPresence, now: i64) -> PresenceApply {
        let node_conflict =
            !self.expected_node_id.trim().is_empty() && self.expected_node_id != presence.node_id;
        let device_conflict = self
            .remote_device_id
            .as_deref()
            .is_some_and(|device| device != presence.device_id);
        let key_conflict = self
            .remote_public_key
            .as_deref()
            .is_some_and(|key| key != presence.public_key);
        if node_conflict || device_conflict || key_conflict {
            self.status = ShareStatus::IdentityConflict;
            self.last_error = Some(if node_conflict {
                "Iroh NodeId passt nicht zum Code".to_string()
            } else {
                "Praesenz nennt ein anderes Geraet als das gepinnte".to_string()
            });
            return PresenceApply::Conflict;
        }
        let mut pinned = false;
        if self.expected_node_id.trim().is_empty() {
            self.expected_node_id = presence.node_id.clone();
            pinned = true;
        }
        if self.remote_device_id.is_none() {
            self.remote_device_id = Some(presence.device_id.clone());
            pinned = true;
        }
        if self.remote_public_key.is_none() {
            self.remote_public_key = Some(presence.public_key.clone());
            pinned = true;
        }
        if presence.is_signed() && !self.relation.signed_presence {
            self.relation.signed_presence = true;
            pinned = true;
        }
        self.last_seen = Some(now);
        self.status = if self.access_state == DirectAccessState::Accepted {
            ShareStatus::Available
        } else {
            ShareStatus::WaitingForAccess
        };
        self.last_error = None;
        self.presence = Some(presence);
        if pinned {
            PresenceApply::Pinned
        } else {
            PresenceApply::Runtime
        }
    }
}
