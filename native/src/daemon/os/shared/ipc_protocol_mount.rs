//! Wire types of the drive host (`--mount-host`): the backend scheme and the
//! configuration that the daemon hands to the host in `MountHostReady`.
//!
//! Moved out of `ipc_protocol.rs` without any change of their serde form
//! (plan 2026-10-17 „Rechte, se Desktop, Tresore“, B2, package R1);
//! `ipc_protocol.rs` re-exports both, so every caller keeps its path. The
//! fixtures in `ipc_protocol_mount_fixtures/` pin the form of v0.5.182
//! (commit `da2646d2`); the only addition since then is
//! `MountHostConfig::path_dialect`.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::daemon) enum MountBackendScheme {
    Local,
    Sftp,
    Ftp,
    Webdav,
    /// Never mounted (`mount_source` refuses SMB); present so the mapping
    /// stays total.
    Smb,
    GoogleDrive,
    Peer,
}

impl From<crate::vfs::Scheme> for MountBackendScheme {
    fn from(value: crate::vfs::Scheme) -> Self {
        match value {
            crate::vfs::Scheme::Local => Self::Local,
            crate::vfs::Scheme::Sftp => Self::Sftp,
            crate::vfs::Scheme::Ftp => Self::Ftp,
            crate::vfs::Scheme::Webdav => Self::Webdav,
            crate::vfs::Scheme::Smb => Self::Smb,
            crate::vfs::Scheme::GDrive => Self::GoogleDrive,
            crate::vfs::Scheme::Peer => Self::Peer,
        }
    }
}

impl From<MountBackendScheme> for crate::vfs::Scheme {
    fn from(value: MountBackendScheme) -> Self {
        match value {
            MountBackendScheme::Local => Self::Local,
            MountBackendScheme::Sftp => Self::Sftp,
            MountBackendScheme::Ftp => Self::Ftp,
            MountBackendScheme::Webdav => Self::Webdav,
            MountBackendScheme::Smb => Self::Smb,
            MountBackendScheme::GoogleDrive => Self::GDrive,
            MountBackendScheme::Peer => Self::Peer,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountHostConfig {
    pub id: crate::mount::MountId,
    pub drive: crate::mount::DriveSelection,
    pub mode: crate::mount::MountMode,
    #[serde(default)]
    pub metadata: crate::mount::MountMetadataPolicy,
    #[serde(default)]
    pub cache: crate::mount::MountCachePolicy,
    #[serde(default)]
    pub runtime_preference: crate::mount::MountRuntimePreference,
    pub label: String,
    /// Name rules of the mount engine (DRV-M1). A configuration without the
    /// field (daemon of v0.5.182 or older) means the Windows rules, the only
    /// rules before the field existed.
    #[serde(default)]
    pub path_dialect: crate::mount::PathDialect,
}

impl MountHostConfig {
    pub fn with_cache_policy(mut self, cache: crate::mount::MountCachePolicy) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_runtime_preference(
        mut self,
        preference: crate::mount::MountRuntimePreference,
    ) -> Self {
        self.runtime_preference = preference;
        self
    }
}

impl From<&crate::mount::MountConfig> for MountHostConfig {
    fn from(config: &crate::mount::MountConfig) -> Self {
        Self {
            id: config.id.clone(),
            drive: config.drive,
            mode: config.mode,
            metadata: config.metadata,
            cache: config.cache,
            runtime_preference: config.runtime_preference,
            label: config.label.clone(),
            // A saved drive carries no dialect: the rules belong to the host
            // platform. Default = the Windows rules of every earlier version;
            // a host platform with other rules sets the public field.
            path_dialect: crate::mount::PathDialect::default(),
        }
    }
}

#[cfg(test)]
#[path = "ipc_protocol_mount_tests.rs"]
mod tests;
