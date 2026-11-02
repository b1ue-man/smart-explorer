use std::io;
use std::sync::{Arc, Mutex};

use crate::vfs::MountPathCapabilities;

use super::fs::{self, ShareExportConfig};
// The peer namespace lives in the storage layer (D13).
pub use crate::vfs::is_synthetic_container;
use crate::vfs::{normalize_share_path as normalize, SHARE_CONNECTIONS_ROOT as CONNECTIONS_ROOT};

pub(super) struct ResolvedMountCapabilities {
    pub(super) virtual_root: String,
    pub(super) target: fs::ResolvedTarget,
    pub(super) capabilities: MountPathCapabilities,
}

impl ResolvedMountCapabilities {
    pub(super) fn lease_root_confined(&self) -> bool {
        self.capabilities.root_confinement.is_enforced()
    }
}

pub(super) fn resolve_mount_capabilities(
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<Option<ResolvedMountCapabilities>> {
    let normalized = normalize(path)?;
    if normalized == "/" || normalized == CONNECTIONS_ROOT {
        // These are synthetic mountpoint containers, not writable namespace
        // targets. Treating them as an intersection of children would falsely
        // permit create/cross-export rename and would fan one capability probe
        // out into credential and network access for every saved connection.
        return Ok(None);
    }
    let target = fs::resolve(&normalized, exports)?;
    let capabilities = target.backend.mount_path_capabilities(&target.path)?;
    Ok(Some(ResolvedMountCapabilities {
        virtual_root: normalized,
        target,
        capabilities,
    }))
}

