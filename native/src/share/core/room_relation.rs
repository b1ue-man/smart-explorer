use std::fmt;

pub const ROOM_RELATION_SECRET_BYTES: usize = 32;
pub const MAX_ROOM_RELATION_ID_BYTES: usize = 128;
pub const MAX_ROOM_DISPLAY_NAME_BYTES: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub struct RoomRelationMaterial {
    room_id: String,
    secret: [u8; ROOM_RELATION_SECRET_BYTES],
}

impl RoomRelationMaterial {
    pub fn new(
        room_id: impl Into<String>,
        mut secret: Vec<u8>,
    ) -> Result<Self, RoomRelationError> {
        let room_id = room_id.into();
        if !valid_identifier(&room_id) {
            secret.fill(0);
            return Err(RoomRelationError::InvalidRoomId);
        }
        if secret.len() != ROOM_RELATION_SECRET_BYTES {
            secret.fill(0);
            return Err(RoomRelationError::InvalidSecretLength);
        }
        let mut durable_secret = [0; ROOM_RELATION_SECRET_BYTES];
        durable_secret.copy_from_slice(&secret);
        secret.fill(0);
        Ok(Self {
            room_id,
            secret: durable_secret,
        })
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub(crate) fn secret(&self) -> &[u8] {
        &self.secret
    }
}

impl fmt::Debug for RoomRelationMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomRelationMaterial")
            .field("room_id", &self.room_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for RoomRelationMaterial {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoomJoinIntent;

#[derive(Clone, PartialEq, Eq)]
pub struct RoomRelationOffer {
    display_name: String,
    material: RoomRelationMaterial,
}

impl RoomRelationOffer {
    pub fn new(
        material: RoomRelationMaterial,
        display_name: impl Into<String>,
    ) -> Result<Self, RoomRelationError> {
        let display_name = display_name.into();
        let display_name = canonical_room_display_name(&display_name)?;
        Ok(Self {
            display_name,
            material,
        })
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn material(&self) -> &RoomRelationMaterial {
        &self.material
    }
}

impl fmt::Debug for RoomRelationOffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomRelationOffer")
            .field("display_name", &self.display_name)
            .field("material", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RoomRelationSnapshot {
    room_profile_id: String,
    offer: RoomRelationOffer,
}

impl RoomRelationSnapshot {
    pub fn new(
        room_profile_id: impl Into<String>,
        offer: RoomRelationOffer,
    ) -> Result<Self, RoomRelationError> {
        let room_profile_id = room_profile_id.into();
        if !valid_identifier(&room_profile_id) {
            return Err(RoomRelationError::InvalidProfileId);
        }
        Ok(Self {
            room_profile_id,
            offer,
        })
    }

    pub fn room_profile_id(&self) -> &str {
        &self.room_profile_id
    }

    pub fn offer(&self) -> &RoomRelationOffer {
        &self.offer
    }

    pub fn into_offer(self) -> RoomRelationOffer {
        self.offer
    }
}

impl fmt::Debug for RoomRelationSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomRelationSnapshot")
            .field("room_profile_id", &self.room_profile_id)
            .field("offer", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomPersistenceOutcome {
    Changed { room_profile_id: String },
    AlreadyComplete { room_profile_id: String },
}

impl RoomPersistenceOutcome {
    pub fn room_profile_id(&self) -> &str {
        match self {
            Self::Changed { room_profile_id } | Self::AlreadyComplete { room_profile_id } => {
                room_profile_id
            }
        }
    }

    pub fn changed(&self) -> bool {
        matches!(self, Self::Changed { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomRelationError {
    InvalidRoomId,
    InvalidProfileId,
    InvalidDisplayName,
    InvalidSecretLength,
}

impl fmt::Display for RoomRelationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRoomId => "invalid Room relation identifier",
            Self::InvalidProfileId => "invalid local Room profile identifier",
            Self::InvalidDisplayName => "invalid Room display name",
            Self::InvalidSecretLength => "Room relation secret must contain 32 bytes",
        })
    }
}

impl std::error::Error for RoomRelationError {}

pub(crate) fn canonical_room_display_name(
    value: &str,
) -> Result<String, RoomRelationError> {
    let trimmed = value.trim();
    let display_name = if trimmed.is_empty() { "Raum" } else { trimmed };
    if display_name.len() > MAX_ROOM_DISPLAY_NAME_BYTES
        || display_name.chars().any(char::is_control)
    {
        return Err(RoomRelationError::InvalidDisplayName);
    }
    Ok(display_name.to_string())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ROOM_RELATION_ID_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
