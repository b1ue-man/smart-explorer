//! Unchanged mount capability wire DTOs and their VFS conversions.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum MountDeleteDisposition {
    Recycle,
    Permanent,
    Unsupported,
}

impl From<crate::vfs::DeleteDisposition> for MountDeleteDisposition {
    fn from(value: crate::vfs::DeleteDisposition) -> Self {
        match value {
            crate::vfs::DeleteDisposition::Recycle => Self::Recycle,
            crate::vfs::DeleteDisposition::Permanent => Self::Permanent,
            crate::vfs::DeleteDisposition::Unsupported => Self::Unsupported,
        }
    }
}

impl From<MountDeleteDisposition> for crate::vfs::DeleteDisposition {
    fn from(value: MountDeleteDisposition) -> Self {
        match value {
            MountDeleteDisposition::Recycle => Self::Recycle,
            MountDeleteDisposition::Permanent => Self::Permanent,
            MountDeleteDisposition::Unsupported => Self::Unsupported,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(in super::super) struct MountBackendCapabilities {
    pub(in super::super) rename_overwrites: bool,
    pub(in super::super) staged_write_create: bool,
    pub(in super::super) staged_write_replace: bool,
    pub(in super::super) staged_namespace_replace: bool,
    #[serde(default)]
    pub(in super::super) case_sensitive_paths: bool,
    pub(in super::super) delete_disposition: MountDeleteDisposition,
    pub(in super::super) parallelism: u8,
}

impl MountBackendCapabilities {
    pub(in super::super) fn from_backend(backend: &crate::vfs::BackendHandle) -> Self {
        let staged_write = backend.staged_write_capabilities("/");
        Self {
            rename_overwrites: staged_write.namespace_replace,
            staged_write_create: staged_write.create,
            staged_write_replace: staged_write.replace,
            staged_namespace_replace: staged_write.namespace_replace,
            case_sensitive_paths: backend.case_sensitive_paths("/"),
            delete_disposition: backend.delete_disposition().into(),
            // The daemon proxy itself admits at most eight active requests.
            parallelism: backend.parallelism().clamp(1, 8) as u8,
        }
    }

    pub(in super::super) fn staged_write(self) -> crate::vfs::StagedWriteCapabilities {
        crate::vfs::StagedWriteCapabilities {
            create: self.staged_write_create,
            replace: self.staged_write_replace,
            namespace_replace: self.staged_namespace_replace,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(in super::super) struct MountPathCapabilitiesWire {
    pub(in super::super) staged_write_create: bool,
    pub(in super::super) staged_write_replace: bool,
    pub(in super::super) staged_namespace_replace: bool,
    pub(in super::super) root_confined: bool,
}

impl From<crate::vfs::MountPathCapabilities> for MountPathCapabilitiesWire {
    fn from(value: crate::vfs::MountPathCapabilities) -> Self {
        Self {
            staged_write_create: value.staged_write.create,
            staged_write_replace: value.staged_write.replace,
            staged_namespace_replace: value.staged_write.namespace_replace,
            root_confined: value.root_confinement.is_enforced(),
        }
    }
}

impl From<MountPathCapabilitiesWire> for crate::vfs::MountPathCapabilities {
    fn from(value: MountPathCapabilitiesWire) -> Self {
        Self {
            staged_write: crate::vfs::StagedWriteCapabilities {
                create: value.staged_write_create,
                replace: value.staged_write_replace,
                namespace_replace: value.staged_namespace_replace,
            },
            root_confinement: if value.root_confined {
                crate::vfs::RootConfinement::Enforced
            } else {
                crate::vfs::RootConfinement::Unverified
            },
        }
    }
}
