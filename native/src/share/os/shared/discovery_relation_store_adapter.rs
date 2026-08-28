use std::fmt;

use super::direct_reciprocal::{DirectReciprocalError, DirectReciprocalPeer};
use super::direct_reciprocal_persistence::{
    persist_reciprocal_direct_peer, DirectReciprocalPersistenceError,
};
use super::discovery_relation_store::{
    canonical_direct_outcome, DiscoveryRelationOutcome, RelationStore, RelationStoreCommit,
    RelationStoreError,
};
use super::profiles::ShareProfiles;
use super::room_relation::{RoomRelationMaterial, RoomRelationOffer, RoomRelationSnapshot};

pub struct SystemRelationStore {
    default_home: Option<String>,
}

impl SystemRelationStore {
    pub fn new(default_home: Option<String>) -> Self {
        Self { default_home }
    }
}

impl fmt::Debug for SystemRelationStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SystemRelationStore")
            .field("default_home", &self.default_home.as_ref().map(|_| "[CONFIGURED]"))
            .finish()
    }
}

impl RelationStore for SystemRelationStore {
    fn load_room(
        &mut self,
        room_profile_id: &str,
    ) -> Result<RoomRelationSnapshot, RelationStoreError> {
        let profiles = ShareProfiles::load_checked(self.default_home.clone())
            .map_err(RelationStoreError::Persistence)?;
        let room = profiles
            .rooms
            .iter()
            .find(|room| room.id == room_profile_id)
            .ok_or_else(|| RelationStoreError::Invalid("Room profile is missing".into()))?;
        let material = ShareProfiles::room_relation_material_checked(room)
            .map_err(RelationStoreError::Persistence)?
            .ok_or_else(|| RelationStoreError::Persistence("Room credential is missing".into()))?;
        let offer = RoomRelationOffer::new(material, room.name.clone())
            .map_err(|error| RelationStoreError::Invalid(error.to_string()))?;
        RoomRelationSnapshot::new(room.id.clone(), offer)
            .map_err(|error| RelationStoreError::Invalid(error.to_string()))
    }

    fn persist_direct(
        &mut self,
        peer: &DirectReciprocalPeer,
    ) -> Result<RelationStoreCommit, RelationStoreError> {
        let result = persist_reciprocal_direct_peer(self.default_home.clone(), peer)
            .map_err(map_direct_error)?;
        let changed = result.outcome().changed();
        let contact_id = result.outcome().contact_id().to_string();
        let (profiles, _) = result.into_parts();
        let outcome = canonical_direct_outcome(&profiles, contact_id)?;
        Ok(RelationStoreCommit::new(profiles, outcome, changed))
    }

    fn persist_room(
        &mut self,
        material: &RoomRelationMaterial,
        display_name: &str,
    ) -> Result<RelationStoreCommit, RelationStoreError> {
        let (profiles, persisted) = ShareProfiles::add_room_material_persisted(
            self.default_home.clone(),
            material,
            display_name,
        )
        .map_err(RelationStoreError::Persistence)?;
        let changed = persisted.changed();
        let room_profile_id = persisted.room_profile_id().to_string();
        let display_name = profiles
            .rooms
            .iter()
            .find(|room| room.id == room_profile_id)
            .map(|room| room.name.clone())
            .ok_or_else(|| {
                RelationStoreError::Persistence(
                    "committed Room profile is missing from its canonical snapshot".into(),
                )
            })?;
        Ok(RelationStoreCommit::new(
            profiles,
            DiscoveryRelationOutcome::RoomInstalled {
                room_profile_id,
                display_name,
            },
            changed,
        ))
    }
}

fn map_direct_error(error: DirectReciprocalPersistenceError) -> RelationStoreError {
    match error {
        DirectReciprocalPersistenceError::Conflict(error) => {
            RelationStoreError::Conflict(DirectReciprocalError::Conflict(error).to_string())
        }
        DirectReciprocalPersistenceError::PolicyDenied(error) => RelationStoreError::PolicyDenied(
            DirectReciprocalError::PolicyDenied(error).to_string(),
        ),
        DirectReciprocalPersistenceError::Invalid(error) => {
            RelationStoreError::Invalid(error.to_string())
        }
        DirectReciprocalPersistenceError::Persistence(error) => {
            RelationStoreError::Persistence(error)
        }
    }
}
