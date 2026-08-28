use std::fmt;

use super::direct_protocol::{validate_direct_lookup_id, DirectPeerIdentity, DirectProtocolError};
use super::profiles::{DirectCode, ShareProfiles};
use super::types::{DirectAccessState, DirectContact, DirectGrant, DirectGrantState};

/// The Direct-code material which is durable for one reciprocal relationship.
/// It deliberately travels with the authenticated identity, rather than as a
/// collection of unrelated pairing arguments.
#[derive(Clone, PartialEq, Eq)]
pub struct DirectRelationMaterial {
    lookup_id: String,
    secret: [u8; 32],
}

impl DirectRelationMaterial {
    pub fn new(
        lookup_id: impl Into<String>,
        mut secret: Vec<u8>,
    ) -> Result<Self, DirectReciprocalError> {
        let lookup_id = lookup_id.into();
        if validate_direct_lookup_id(&lookup_id).is_err() {
            secret.fill(0);
            return Err(DirectReciprocalError::InvalidMaterial(
                "direct lookup id is invalid",
            ));
        }
        if secret.len() != 32 {
            secret.fill(0);
            return Err(DirectReciprocalError::InvalidMaterial(
                "direct relation secret must contain 32 bytes",
            ));
        }
        let mut durable_secret = [0; 32];
        durable_secret.copy_from_slice(&secret);
        secret.fill(0);
        let secret = durable_secret;
        Ok(Self { lookup_id, secret })
    }

    pub fn lookup_id(&self) -> &str {
        &self.lookup_id
    }

    pub(crate) fn secret(&self) -> &[u8] {
        &self.secret
    }
}

impl fmt::Debug for DirectRelationMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectRelationMaterial")
            .field("lookup_id", &self.lookup_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for DirectRelationMaterial {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

/// Exact authenticated peer identity plus the Direct-code material that names
/// and authorizes the durable relationship.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectReciprocalPeer {
    identity: DirectPeerIdentity,
    material: DirectRelationMaterial,
}

impl DirectReciprocalPeer {
    pub fn authenticated(
        identity: DirectPeerIdentity,
        material: DirectRelationMaterial,
    ) -> Result<Self, DirectReciprocalError> {
        identity
            .validate()
            .map_err(DirectReciprocalError::InvalidIdentity)?;
        Ok(Self { identity, material })
    }

    pub(crate) fn from_direct_code(
        identity: DirectPeerIdentity,
        code: &str,
    ) -> Result<Self, DirectReciprocalError> {
        identity
            .validate()
            .map_err(DirectReciprocalError::InvalidIdentity)?;
        let mut parsed = DirectCode::parse(code)
            .map_err(|_| DirectReciprocalError::InvalidMaterial("invalid Direct code"))?;
        if parsed.fingerprint != identity.fingerprint || parsed.node_id != identity.node_id {
            return Err(DirectReciprocalError::InvalidMaterial(
                "Direct code identity pins do not match the authenticated peer",
            ));
        }
        let lookup_id = std::mem::take(&mut parsed.lookup_id);
        let secret = std::mem::take(&mut parsed.secret);
        Ok(Self {
            identity,
            material: DirectRelationMaterial::new(lookup_id, secret)?,
        })
    }

    pub fn identity(&self) -> &DirectPeerIdentity {
        &self.identity
    }

    pub fn material(&self) -> &DirectRelationMaterial {
        &self.material
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectReciprocalConflict {
    ContactIdentity { device_id: String },
    GrantIdentity { device_id: String },
    RelationMaterial { device_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectReciprocalError {
    InvalidIdentity(DirectProtocolError),
    InvalidMaterial(&'static str),
    Conflict(DirectReciprocalConflict),
}

impl fmt::Display for DirectReciprocalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity(error) => {
                write!(formatter, "invalid authenticated Direct peer: {error}")
            }
            Self::InvalidMaterial(error) => {
                write!(formatter, "invalid Direct relation material: {error}")
            }
            Self::Conflict(DirectReciprocalConflict::ContactIdentity { device_id }) => {
                write!(
                    formatter,
                    "Direct contact identity conflicts for device {device_id}"
                )
            }
            Self::Conflict(DirectReciprocalConflict::GrantIdentity { device_id }) => {
                write!(
                    formatter,
                    "Direct grant identity conflicts for device {device_id}"
                )
            }
            Self::Conflict(DirectReciprocalConflict::RelationMaterial { device_id }) => {
                write!(
                    formatter,
                    "Direct relation material conflicts for device {device_id}"
                )
            }
        }
    }
}

impl std::error::Error for DirectReciprocalError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectReciprocalApply {
    Changed { contact_id: String },
    AlreadyComplete { contact_id: String },
}

impl ShareProfiles {
    /// Applies the profile-only half of a reciprocal Direct installation.
    /// Callers must run this inside the persisted CAS transaction; it has no
    /// external side effects and is therefore safe to replay after a rebase.
    pub(crate) fn apply_reciprocal_direct_peer(
        &mut self,
        peer: &DirectReciprocalPeer,
        new_contact_id: &str,
        now: i64,
    ) -> Result<DirectReciprocalApply, DirectReciprocalError> {
        let identity = peer.identity();
        let material = peer.material();
        let contact_index = self.reciprocal_contact_index(peer)?;
        let contact_id = match contact_index {
            Some(index) => self.direct_contacts[index].id.clone(),
            None => new_contact_id.to_string(),
        };
        if contact_index.is_none()
            && self
                .direct_contacts
                .iter()
                .any(|contact| contact.id == new_contact_id)
        {
            return Err(DirectReciprocalError::Conflict(
                DirectReciprocalConflict::RelationMaterial {
                    device_id: identity.device_id.clone(),
                },
            ));
        }

        let mut grant_index = None;
        for (index, grant) in self.direct_grants.iter().enumerate() {
            if grant.device_id != identity.device_id {
                continue;
            }
            if grant_index.replace(index).is_some() || !grant_matches(grant, identity) {
                return Err(DirectReciprocalError::Conflict(
                    DirectReciprocalConflict::GrantIdentity {
                        device_id: identity.device_id.clone(),
                    },
                ));
            }
        }

        let mut changed = false;
        if let Some(index) = contact_index {
            let contact = &mut self.direct_contacts[index];
            if contact.expected_node_id.is_empty() {
                contact.expected_node_id = identity.node_id.clone();
                changed = true;
            }
            if contact.remote_device_id.as_deref() != Some(identity.device_id.as_str()) {
                contact.remote_device_id = Some(identity.device_id.clone());
                changed = true;
            }
            if contact.remote_public_key.as_deref() != Some(identity.public_key.as_str()) {
                contact.remote_public_key = Some(identity.public_key.clone());
                changed = true;
            }
            if contact.access_state != DirectAccessState::Accepted {
                contact.access_state = DirectAccessState::Accepted;
                changed = true;
            }
            if contact.accepted_at.is_none() {
                contact.accepted_at = Some(now);
                changed = true;
            }
            if contact.accepted_public_key.as_deref() != Some(identity.public_key.as_str()) {
                contact.accepted_public_key = Some(identity.public_key.clone());
                changed = true;
            }
            // Existing presentation, preferences, presence, errors, and request
            // history are intentionally retained: authenticated pairing is not
            // authority to erase concurrent local state.
        } else {
            self.direct_contacts.push(DirectContact {
                id: contact_id.clone(),
                display_name: identity.device_name.clone(),
                lookup_id: material.lookup_id().to_string(),
                expected_fingerprint: identity.fingerprint.clone(),
                expected_node_id: identity.node_id.clone(),
                remote_device_id: Some(identity.device_id.clone()),
                remote_public_key: Some(identity.public_key.clone()),
                auto_connect: true,
                auto_open: false,
                last_seen: None,
                status: Default::default(),
                last_error: None,
                presence: None,
                access_state: DirectAccessState::Accepted,
                request_sent_at: None,
                accepted_at: Some(now),
                accepted_public_key: Some(identity.public_key.clone()),
            });
            changed = true;
        }

        if let Some(index) = grant_index {
            let grant = &mut self.direct_grants[index];
            if grant.state != DirectGrantState::Accepted {
                grant.state = DirectGrantState::Accepted;
                grant.updated_at = now;
                changed = true;
            }
        } else {
            self.direct_grants.push(DirectGrant {
                device_id: identity.device_id.clone(),
                device_name: identity.device_name.clone(),
                public_key: identity.public_key.clone(),
                fingerprint: identity.fingerprint.clone(),
                node_id: identity.node_id.clone(),
                state: DirectGrantState::Accepted,
                updated_at: now,
                exec: Default::default(),
            });
            changed = true;
        }

        Ok(if changed {
            DirectReciprocalApply::Changed { contact_id }
        } else {
            DirectReciprocalApply::AlreadyComplete { contact_id }
        })
    }

    fn reciprocal_contact_index(
        &self,
        peer: &DirectReciprocalPeer,
    ) -> Result<Option<usize>, DirectReciprocalError> {
        let identity = peer.identity();
        let material = peer.material();
        let mut matched = None;
        for (index, contact) in self.direct_contacts.iter().enumerate() {
            let same_device =
                contact.remote_device_id.as_deref() == Some(identity.device_id.as_str());
            let same_material = contact.lookup_id == material.lookup_id();
            if !same_device && !same_material {
                continue;
            }
            if !contact_matches(contact, identity, material) {
                let conflict = if same_device {
                    DirectReciprocalConflict::ContactIdentity {
                        device_id: identity.device_id.clone(),
                    }
                } else {
                    DirectReciprocalConflict::RelationMaterial {
                        device_id: identity.device_id.clone(),
                    }
                };
                return Err(DirectReciprocalError::Conflict(conflict));
            }
            if matched.replace(index).is_some() {
                return Err(DirectReciprocalError::Conflict(
                    DirectReciprocalConflict::RelationMaterial {
                        device_id: identity.device_id.clone(),
                    },
                ));
            }
        }
        Ok(matched)
    }
}

fn contact_matches(
    contact: &DirectContact,
    identity: &DirectPeerIdentity,
    material: &DirectRelationMaterial,
) -> bool {
    contact.lookup_id == material.lookup_id()
        && contact.expected_fingerprint == identity.fingerprint
        && (contact.expected_node_id.is_empty() || contact.expected_node_id == identity.node_id)
        && contact
            .remote_device_id
            .as_deref()
            .map_or(true, |device_id| device_id == identity.device_id)
        && contact
            .remote_public_key
            .as_deref()
            .map_or(true, |public_key| public_key == identity.public_key)
        && contact
            .accepted_public_key
            .as_deref()
            .map_or(true, |public_key| public_key == identity.public_key)
}

fn grant_matches(grant: &DirectGrant, identity: &DirectPeerIdentity) -> bool {
    grant.public_key == identity.public_key
        && grant.fingerprint == identity.fingerprint
        && grant.node_id == identity.node_id
}
