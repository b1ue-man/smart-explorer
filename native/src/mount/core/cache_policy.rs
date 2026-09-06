//! Persistent limits for disposable mounted-drive data and runtime selection.

use serde::{de, Deserialize, Deserializer, Serialize};
use std::io;

pub const DEFAULT_MOUNT_CACHE_MIB: u32 = 500;
pub const MAX_MOUNT_CACHE_MIB: u32 = 65_536;

/// Per-drive idle, clean content budget. Open and unsaved data is excluded:
/// it must never be discarded merely to satisfy this retention limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MountCachePolicy {
    retained_mib: u32,
}

impl Default for MountCachePolicy {
    fn default() -> Self {
        Self {
            retained_mib: DEFAULT_MOUNT_CACHE_MIB,
        }
    }
}

impl MountCachePolicy {
    /// Zero disables retention after the last active user releases the file.
    pub fn new(retained_mib: u32) -> io::Result<Self> {
        let policy = Self { retained_mib };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(self) -> io::Result<()> {
        if self.retained_mib > MAX_MOUNT_CACHE_MIB {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("retained cache must be between 0 and {MAX_MOUNT_CACHE_MIB} MiB"),
            ));
        }
        Ok(())
    }

    pub const fn retained_mib(self) -> u32 {
        self.retained_mib
    }

    pub const fn retained_bytes(self) -> u64 {
        (self.retained_mib as u64) * 1024 * 1024
    }
}

impl<'de> Deserialize<'de> for MountCachePolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            retained_mib: u32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.retained_mib).map_err(de::Error::custom)
    }
}

/// Automatic private-runtime selection can always be bypassed for compatibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountRuntimePreference {
    #[default]
    Auto,
    System,
}
