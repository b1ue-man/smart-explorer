use serde::{Deserialize, Deserializer, Serialize};

use super::types::{MountConfig, MountStatus};

/// Durable boundaries exercised by the single task suite. Fault injection
/// exists only in test binaries and is consumed once on the calling thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryCommitPoint {
    PayloadSynced,
    ManifestSynced,
    PackagePublished,
    OwnershipSynced,
    OwnershipPublished,
}

#[cfg(test)]
std::thread_local! {
    static RECOVERY_FAULT: std::cell::Cell<Option<RecoveryCommitPoint>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_recovery_at(point: Option<RecoveryCommitPoint>) {
    RECOVERY_FAULT.with(|fault| fault.set(point));
}

pub(crate) fn recovery_checkpoint(point: RecoveryCommitPoint) -> std::io::Result<()> {
    #[cfg(test)]
    if RECOVERY_FAULT.with(|fault| {
        if fault.get() == Some(point) { fault.set(None); true } else { false }
    }) {
        return Err(std::io::Error::other(format!("Injected recovery boundary failure: {point:?}")));
    }
    let _ = point;
    Ok(())
}

/// Daemon-owned knowledge about retryable local mounted-drive state.
/// `Unknown` is deliberately retained until the isolated host can audit the
/// journal under its exclusive cache lease.
///
/// `RequiresUnlock` (DRV-M7, contract V2): a vault drive keeps `pending`
/// sealed changes, journal records and namespace intents that only the
/// unlocked vault can open; they are published after the next unlock. Wire
/// form `{"requires_unlock":{"pending":<n>}}`; the unit values keep theirs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountRecovery {
    Clean,
    Required,
    #[default]
    Unknown,
    #[serde(rename = "requires_unlock")]
    RequiresUnlock {
        pending: u32,
    },
}

impl MountRecovery {
    pub const fn is_clean(self) -> bool {
        matches!(self, Self::Clean)
    }

    /// A locked vault with a known count from an authorized engine. Opaque
    /// offline ciphertext cannot supply this count and must stay Unknown.
    pub fn awaiting_unlock(pending: usize) -> Self {
        match pending {
            0 => Self::Clean,
            count => Self::RequiresUnlock {
                pending: u32::try_from(count).unwrap_or(u32::MAX),
            },
        }
    }

    /// Sealed changes that wait for the vault's unlock, if any.
    pub const fn pending_unlock(self) -> Option<u32> {
        match self {
            Self::RequiresUnlock { pending } => Some(pending),
            _ => None,
        }
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
    #[serde(default)]
    pub capacity: Option<super::capacity::MountCapacityReport>,
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
            capacity: Option<super::capacity::MountCapacityReport>,
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
            capacity: wire.capacity,
            recovery,
            recovery_required_compat: recovery.requires_retention(),
        })
    }
}
