//! Where a drive's data comes from. Moved out of `types.rs` without any change
//! of the serde form (DRV-M1); `types.rs` re-exports both types, so every
//! caller keeps its path.
//!
//! DRV-M3 adds the kinds of this batch: the device tree (`Devices`, DRV-M6)
//! and a Cryptomator vault by its site id (`Vault { site_id }`, contract V2,
//! decision S3 OE3). Version 0.5.182 cannot read them, so the registry keeps
//! them only in `registry-2.json` (`stored_in_first_registry`).
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::OnceLock;

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
    /// The device tree „Verbundene Geräte als Ordner“ (DRV-M6); root `/`.
    Devices,
    /// An unlocked Cryptomator vault of strand S3 (contract V2): only the
    /// site id is stored; root `/`.
    Vault {
        site_id: String,
    },
}

impl MountSource {
    pub fn root(&self) -> &BackendRoot {
        match self {
            Self::SavedRemote { root, .. }
            | Self::GoogleDrive { root, .. }
            | Self::Peer { root, .. } => root,
            Self::Devices | Self::Vault { .. } => top_root(),
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
            Self::Devices => Ok(()),
            Self::Vault { site_id } => validate_identity(site_id, "vault site id"),
        }
    }

    /// Kinds that version 0.5.182 reads: they stay in `registry.json`. Every
    /// later kind lives only in `registry-2.json` (strand S2 C4), so a
    /// rolled-back version keeps reading its registry.
    pub fn stored_in_first_registry(&self) -> bool {
        matches!(
            self,
            Self::SavedRemote { .. } | Self::GoogleDrive { .. } | Self::Peer { .. }
        )
    }

    pub fn is_device_tree(&self) -> bool {
        matches!(self, Self::Devices)
    }

    /// Whether mounting reaches other devices through Smart Explorer Share.
    /// A worker started only for the drive restore at login starts no Share
    /// host (decision S2 K7), so such drives wait for the first other
    /// trigger.
    pub fn uses_share(&self) -> bool {
        matches!(self, Self::Peer { .. } | Self::Devices)
    }
}

fn top_root() -> &'static BackendRoot {
    static TOP: OnceLock<BackendRoot> = OnceLock::new();
    TOP.get_or_init(BackendRoot::top)
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
