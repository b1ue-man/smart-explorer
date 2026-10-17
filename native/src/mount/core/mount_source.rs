//! Where a drive's data comes from. Moved out of `types.rs` without any change
//! of the serde form (DRV-M1); `types.rs` re-exports both types, so every
//! caller keeps its path. Later sources (`Devices`, `Vault`) are added here.
use serde::{Deserialize, Serialize};
use std::io;

use super::types::BackendRoot;

const MAX_ID_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerMountTarget {
    Direct { contact_id: String },
    RoomDevice { room_id: String, device_id: String },
}

impl PeerMountTarget {
    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::Direct { contact_id } => validate_identity(contact_id, "contact id"),
            Self::RoomDevice { room_id, device_id } => {
                validate_identity(room_id, "room id")?;
                validate_identity(device_id, "device id")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountSource {
    SavedRemote {
        account: String,
        root: BackendRoot,
    },
    GoogleDrive {
        account: String,
        root: BackendRoot,
    },
    Peer {
        target: PeerMountTarget,
        root: BackendRoot,
    },
}

impl MountSource {
    pub fn root(&self) -> &BackendRoot {
        match self {
            Self::SavedRemote { root, .. }
            | Self::GoogleDrive { root, .. }
            | Self::Peer { root, .. } => root,
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::SavedRemote { account, .. } => validate_identity(account, "saved account"),
            Self::GoogleDrive { account, .. } => {
                validate_identity(account, "Google Drive account")?;
                if account != "cloud:gdrive" {
                    return Err(invalid("unsupported Google Drive account identity"));
                }
                Ok(())
            }
            Self::Peer { target, .. } => target.validate(),
        }
    }
}

fn validate_identity(value: &str, name: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID_LEN
        || value
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(invalid(format!("invalid {name}")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
