use serde::{Deserialize, Deserializer, Serialize};

use super::types::{MountConfig, MountStatus};

/// Daemon-owned knowledge about retryable local mounted-drive state.
/// `Unknown` is deliberately retained until the isolated host can audit the
/// journal under its exclusive cache lease.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountRecovery {
    Clean,
    Required,
    #[default]
    Unknown,
}

impl MountRecovery {
    pub const fn is_clean(self) -> bool {
        matches!(self, Self::Clean)
    }

    pub const fn requires_retention(self) -> bool {
        !self.is_clean()
    }

    pub const fn from_required(required: bool) -> Self {
        if required {
            Self::Required
        } else {
            Self::Clean
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MountSnapshot {
    pub config: MountConfig,
    pub status: MountStatus,
    /// Only `Clean` permits removing registry/cache ownership. `Unknown` and
    /// `Required` retain it for audit, Retry, or manual conflict recovery.
    #[serde(default)]
    pub recovery: MountRecovery,
    /// Wire compatibility for an older GUI/daemon during worker replacement.
    #[serde(default, rename = "recovery_required")]
    pub(crate) recovery_required_compat: bool,
}

impl<'de> Deserialize<'de> for MountSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            config: MountConfig,
            status: MountStatus,
            #[serde(default)]
            recovery: Option<MountRecovery>,
            #[serde(default, rename = "recovery_required")]
            recovery_required_compat: Option<bool>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let recovery = wire.recovery.unwrap_or_else(|| {
            wire.recovery_required_compat
                .map(MountRecovery::from_required)
                .unwrap_or(MountRecovery::Unknown)
        });
        Ok(Self {
            config: wire.config,
            status: wire.status,
            recovery,
            recovery_required_compat: recovery.requires_retention(),
        })
    }
}
