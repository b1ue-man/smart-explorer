//! Provider space is independent from the local transfer/mount cache budget.
use super::{Backend, VfsResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapacityState {
    ProviderKnown,
    AccountQuota,
    Unbounded,
    Unknown,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapacitySource {
    Unknown,
    Filesystem,
    SftpStatvfs,
    WebDavQuota,
    DriveAccount,
    Share,
    Agent,
    Archive,
    CacheCompatibility,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapacityFailure {
    Authorization,
    Offline,
    Temporary,
    InvalidResponse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacitySnapshot {
    pub state: CapacityState,
    pub total_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub source: CapacitySource,
    pub failure: Option<CapacityFailure>,
    pub observed_at_ms: i64,
    /// Backend/account identity; never a frontend cache volume identity.
    pub identity: String,
    pub root: String,
    pub read_only: bool,
}
impl CapacitySnapshot {
    pub fn unknown(identity: String, root: &str) -> Self {
        Self {
            state: CapacityState::Unknown,
            total_bytes: None,
            used_bytes: None,
            available_bytes: None,
            source: CapacitySource::Unknown,
            failure: None,
            observed_at_ms: observed_now_ms(),
            identity,
            root: root.into(),
            read_only: false,
        }
    }
    pub fn known(
        identity: String,
        root: &str,
        source: CapacitySource,
        total: u64,
        used: u64,
        available: u64,
    ) -> Self {
        let mut value = Self::unknown(identity, root);
        value.state = CapacityState::ProviderKnown;
        value.total_bytes = Some(total);
        value.used_bytes = Some(used);
        value.available_bytes = Some(available);
        value.source = source;
        value.observed_at_ms = observed_now_ms();
        value
    }
    pub fn unavailable(identity: String, root: &str, source: CapacitySource) -> Self {
        let mut value = Self::unknown(identity, root);
        value.state = CapacityState::Unavailable;
        value.failure = Some(CapacityFailure::Temporary);
        value.source = source;
        value.observed_at_ms = observed_now_ms();
        value
    }
}
pub fn observed_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
/// Blocking metadata query: callers must run this outside rendering/FS callbacks.
pub fn capacity<B: Backend + ?Sized>(backend: &B, root: &str) -> VfsResult<CapacitySnapshot> {
    if root.len() > 32768 || root.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid capacity root",
        ));
    }
    let result = match backend.extensions() {
        Some(extension) => extension.capacity(root),
        None => Ok(CapacitySnapshot::unknown(backend.state_identity(), root)),
    };
    result.and_then(|value| {
        value.validate()?;
        Ok(value)
    })
}

impl CapacitySnapshot {
    pub fn validate(&self) -> VfsResult<()> {
        let invalid = self.identity.len() > 4096
            || self.root.len() > 32768
            || ((self.state == CapacityState::Unavailable) != self.failure.is_some())
            || self.identity.contains('\0')
            || self.root.contains('\0')
            || matches!((self.total_bytes, self.available_bytes), (Some(total), Some(available)) if available > total)
            || (self.state == CapacityState::Unbounded
                && (self.total_bytes.is_some() || self.available_bytes.is_some()))
            || (matches!(
                self.state,
                CapacityState::Unknown | CapacityState::Unavailable
            ) && (self.total_bytes.is_some() || self.available_bytes.is_some()));
        if invalid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid capacity metadata",
            ));
        }
        Ok(())
    }
}

pub fn capacity_failure(
    identity: String,
    root: &str,
    source: CapacitySource,
    error: &std::io::Error,
) -> CapacitySnapshot {
    let mut value = CapacitySnapshot::unavailable(identity, root, source);
    value.failure = Some(match error.kind() {
        std::io::ErrorKind::PermissionDenied => CapacityFailure::Authorization,
        std::io::ErrorKind::NotConnected | std::io::ErrorKind::HostUnreachable => {
            CapacityFailure::Offline
        }
        std::io::ErrorKind::InvalidData => CapacityFailure::InvalidResponse,
        _ => CapacityFailure::Temporary,
    });
    value
}
