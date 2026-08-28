use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::direct_reciprocal::{
    DirectReciprocalApply, DirectReciprocalError, DirectReciprocalPeer, DirectRelationMaterial,
};
use super::profiles::ShareProfiles;
use super::room_relation::{
    canonical_room_display_name, RoomRelationMaterial, RoomRelationOffer, RoomRelationSnapshot,
};
use super::types::{RoomProfile, ShareStatus};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DiscoveryRelationOutcome {
    DirectInstalled {
        contact_id: String,
        display_name: String,
    },
    RoomInstalled {
        room_profile_id: String,
        display_name: String,
    },
    RoomShared {
        room_profile_id: String,
        display_name: String,
    },
}

pub struct RelationStoreCommit {
    profiles: ShareProfiles,
    outcome: DiscoveryRelationOutcome,
    changed: bool,
}

impl RelationStoreCommit {
    pub fn new(
        profiles: ShareProfiles,
        outcome: DiscoveryRelationOutcome,
        changed: bool,
    ) -> Self {
        Self {
            profiles,
            outcome,
            changed,
        }
    }

    pub fn profiles(&self) -> &ShareProfiles {
        &self.profiles
    }

    pub fn outcome(&self) -> &DiscoveryRelationOutcome {
        &self.outcome
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    pub fn into_parts(self) -> (ShareProfiles, DiscoveryRelationOutcome, bool) {
        (self.profiles, self.outcome, self.changed)
    }
}

pub(crate) fn canonical_direct_outcome(
    profiles: &ShareProfiles,
    contact_id: String,
) -> Result<DiscoveryRelationOutcome, RelationStoreError> {
    let display_name = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id.as_str() == contact_id.as_str())
        .map(|contact| contact.display_name.clone())
        .ok_or_else(|| {
            RelationStoreError::Persistence(
                "committed Direct contact is missing from its canonical snapshot".into(),
            )
        })?;
    Ok(DiscoveryRelationOutcome::DirectInstalled {
        contact_id,
        display_name,
    })
}

impl fmt::Debug for RelationStoreCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationStoreCommit")
            .field("profiles", &"[CANONICAL SNAPSHOT]")
            .field("outcome", &self.outcome)
            .field("changed", &self.changed)
            .finish()
    }
}

pub trait RelationStore: Send {
    fn load_room(
        &mut self,
        room_profile_id: &str,
    ) -> Result<RoomRelationSnapshot, RelationStoreError>;

    fn persist_direct(
        &mut self,
        peer: &DirectReciprocalPeer,
    ) -> Result<RelationStoreCommit, RelationStoreError>;

    fn persist_room(
        &mut self,
        material: &RoomRelationMaterial,
        display_name: &str,
    ) -> Result<RelationStoreCommit, RelationStoreError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelationStoreError {
    Unavailable(String),
    Invalid(String),
    Conflict(String),
    PolicyDenied(String),
    Persistence(String),
}

impl fmt::Display for RelationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(error) => write!(formatter, "relation store unavailable: {error}"),
            Self::Invalid(error) => write!(formatter, "invalid relation material: {error}"),
            Self::Conflict(error) => write!(formatter, "relation conflicts with local state: {error}"),
            Self::PolicyDenied(error) => write!(formatter, "relation denied by local policy: {error}"),
            Self::Persistence(error) => write!(formatter, "relation persistence failed: {error}"),
        }
    }
}

impl std::error::Error for RelationStoreError {}

pub struct UnavailableRelationStore {
    reason: String,
}

impl UnavailableRelationStore {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Default for UnavailableRelationStore {
    fn default() -> Self {
        Self::new("no durable relation store was configured")
    }
}

impl fmt::Debug for UnavailableRelationStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnavailableRelationStore")
            .field("reason", &self.reason)
            .finish()
    }
}

impl RelationStore for UnavailableRelationStore {
    fn load_room(
        &mut self,
        _room_profile_id: &str,
    ) -> Result<RoomRelationSnapshot, RelationStoreError> {
        Err(RelationStoreError::Unavailable(self.reason.clone()))
    }

    fn persist_direct(
        &mut self,
        _peer: &DirectReciprocalPeer,
    ) -> Result<RelationStoreCommit, RelationStoreError> {
        Err(RelationStoreError::Unavailable(self.reason.clone()))
    }

    fn persist_room(
        &mut self,
        _material: &RoomRelationMaterial,
        _display_name: &str,
    ) -> Result<RelationStoreCommit, RelationStoreError> {
        Err(RelationStoreError::Unavailable(self.reason.clone()))
    }
}

pub struct InMemoryRelationStore {
    profiles: ShareProfiles,
    direct_material: HashMap<String, DirectRelationMaterial>,
    room_material: HashMap<String, RoomRelationMaterial>,
    next_id: u64,
}

impl InMemoryRelationStore {
    pub fn new(profiles: ShareProfiles) -> Self {
        Self {
            profiles,
            direct_material: HashMap::new(),
            room_material: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn profiles(&self) -> &ShareProfiles {
        &self.profiles
    }

    pub fn seed_direct_material(
        &mut self,
        contact_id: &str,
        material: DirectRelationMaterial,
    ) -> Result<(), RelationStoreError> {
        let contact = self
            .profiles
            .direct_contacts
            .iter()
            .find(|contact| contact.id == contact_id)
            .ok_or_else(|| RelationStoreError::Invalid("Direct contact is missing".into()))?;
        if contact.lookup_id != material.lookup_id() {
            return Err(RelationStoreError::Conflict(
                "Direct contact lookup id does not match its credential".into(),
            ));
        }
        self.direct_material.insert(contact_id.to_string(), material);
        Ok(())
    }

    pub fn seed_room_material(
        &mut self,
        room_profile_id: &str,
        material: RoomRelationMaterial,
    ) -> Result<(), RelationStoreError> {
        let room = self
            .profiles
            .rooms
            .iter()
            .find(|room| room.id == room_profile_id)
            .ok_or_else(|| RelationStoreError::Invalid("Room profile is missing".into()))?;
        if room.room_id != material.room_id() {
            return Err(RelationStoreError::Conflict(
                "Room profile id does not match its credential".into(),
            ));
        }
        self.room_material
            .insert(room_profile_id.to_string(), material);
        Ok(())
    }

    fn next_available_id(&mut self, prefix: &str) -> Result<String, RelationStoreError> {
        loop {
            let id = format!("memory-{prefix}-{}", self.next_id);
            self.next_id = self.next_id.checked_add(1).ok_or_else(|| {
                RelationStoreError::Persistence("in-memory relation id space exhausted".into())
            })?;
            let occupied = self
                .profiles
                .direct_contacts
                .iter()
                .any(|contact| contact.id == id)
                || self.profiles.rooms.iter().any(|room| room.id == id);
            if !occupied {
                return Ok(id);
            }
        }
    }
}

impl Default for InMemoryRelationStore {
    fn default() -> Self {
        Self::new(ShareProfiles::default())
    }
}

impl fmt::Debug for InMemoryRelationStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InMemoryRelationStore")
            .field("direct_credentials", &self.direct_material.len())
            .field("room_credentials", &self.room_material.len())
            .finish_non_exhaustive()
    }
}

impl RelationStore for InMemoryRelationStore {
    fn load_room(
        &mut self,
        room_profile_id: &str,
    ) -> Result<RoomRelationSnapshot, RelationStoreError> {
        let room = self
            .profiles
            .rooms
            .iter()
            .find(|room| room.id == room_profile_id)
            .ok_or_else(|| RelationStoreError::Invalid("Room profile is missing".into()))?;
        let material = self
            .room_material
            .get(room_profile_id)
            .ok_or_else(|| RelationStoreError::Persistence("Room credential is missing".into()))?;
        if room.room_id != material.room_id() {
            return Err(RelationStoreError::Conflict(
                "Room profile conflicts with its credential".into(),
            ));
        }
        let offer = RoomRelationOffer::new(material.clone(), room.name.clone())
            .map_err(|error| RelationStoreError::Invalid(error.to_string()))?;
        RoomRelationSnapshot::new(room_profile_id.to_string(), offer)
            .map_err(|error| RelationStoreError::Invalid(error.to_string()))
    }

    fn persist_direct(
        &mut self,
        peer: &DirectReciprocalPeer,
    ) -> Result<RelationStoreCommit, RelationStoreError> {
        let generated_id = self.next_available_id("direct")?;
        let mut candidate = self.profiles.clone();
        let applied = candidate
            .apply_reciprocal_direct_peer(peer, &generated_id, super::core::now_secs())
            .map_err(map_direct_domain_error)?;
        let (contact_id, changed) = match applied {
            DirectReciprocalApply::Changed { contact_id } => (contact_id, true),
            DirectReciprocalApply::AlreadyComplete { contact_id } => (contact_id, false),
        };
        if let Some(existing) = self.direct_material.get(&contact_id) {
            if existing != peer.material() {
                return Err(RelationStoreError::Conflict(
                    "Direct credential differs from the authenticated peer".into(),
                ));
            }
        } else if contact_id == generated_id {
            self.direct_material
                .insert(contact_id.clone(), peer.material().clone());
        } else {
            return Err(RelationStoreError::Persistence(
                "existing Direct credential is missing".into(),
            ));
        }
        self.profiles = candidate;
        let outcome = canonical_direct_outcome(&self.profiles, contact_id)?;
        Ok(RelationStoreCommit::new(self.profiles.clone(), outcome, changed))
    }

    fn persist_room(
        &mut self,
        material: &RoomRelationMaterial,
        display_name: &str,
    ) -> Result<RelationStoreCommit, RelationStoreError> {
        let display_name = canonical_room_display_name(display_name)
            .map_err(|error| RelationStoreError::Invalid(error.to_string()))?;
        if let Some(room) = self
            .profiles
            .rooms
            .iter()
            .find(|room| room.room_id == material.room_id())
        {
            let existing_material = self
                .room_material
                .get(&room.id)
                .ok_or_else(|| RelationStoreError::Persistence("Room credential is missing".into()))?;
            if existing_material != material {
                return Err(RelationStoreError::Conflict(
                    "Room credential differs from the authenticated offer".into(),
                ));
            }
            return Ok(RelationStoreCommit::new(
                self.profiles.clone(),
                DiscoveryRelationOutcome::RoomInstalled {
                    room_profile_id: room.id.clone(),
                    display_name: room.name.clone(),
                },
                false,
            ));
        }

        let room_profile_id = self.next_available_id("room")?;
        self.profiles.rooms.push(RoomProfile {
            id: room_profile_id.clone(),
            name: display_name.clone(),
            room_id: material.room_id().to_string(),
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: Vec::new(),
            exports: self.profiles.default_direct_exports.clone(),
        });
        self.room_material
            .insert(room_profile_id.clone(), material.clone());
        Ok(RelationStoreCommit::new(
            self.profiles.clone(),
            DiscoveryRelationOutcome::RoomInstalled {
                room_profile_id,
                display_name,
            },
            true,
        ))
    }
}

fn map_direct_domain_error(error: DirectReciprocalError) -> RelationStoreError {
    match error {
        DirectReciprocalError::PolicyDenied(error) => {
            RelationStoreError::PolicyDenied(
                DirectReciprocalError::PolicyDenied(error).to_string(),
            )
        }
        DirectReciprocalError::Conflict(error) => {
            RelationStoreError::Conflict(DirectReciprocalError::Conflict(error).to_string())
        }
        other => RelationStoreError::Invalid(other.to_string()),
    }
}
