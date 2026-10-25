//! Capacity is negotiated, authorized read metadata of one resolved export.
use super::{backend::PeerBackend, fs_access::FsAccess, peer_stream, wire::{FsRequest, FsResponse}};
use crate::vfs::{Backend, CapacitySnapshot, CapacitySource};
use std::io;
pub(super) fn client(backend: &PeerBackend, root: &str) -> io::Result<CapacitySnapshot> {
    let result = (|| {
        if !peer_stream::features(backend, root)?.capacity_v1 {
            return Ok(CapacitySnapshot::unknown(backend.state_identity(), root));
        }
        match backend.request(FsRequest::Capacity { path: root.into() })? {
            FsResponse::Capacity { capacity } => Ok(capacity),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid peer capacity reply")),
        }
    })();
    match result {
        Ok(value) => Ok(value),
        Err(error) => Ok(crate::vfs::capacity_failure(backend.state_identity(), root, CapacitySource::Share, &error)),
    }
}
pub(super) fn host(access: &FsAccess, root: &str) -> io::Result<CapacitySnapshot> {
    access.check_read()?;
    if super::fs_capabilities::is_synthetic_container(root) {
        return Ok(CapacitySnapshot::unknown(String::new(), root));
    }
    let target = access.resolve(root)?;
    let value = crate::vfs::capacity(&*target.backend, &target.path)?;
    access.check_read()?;
    // Re-resolve under the current grant after network/provider work.
    let checked = access.resolve(root)?;
    if checked.mount_key != target.mount_key || checked.path != target.path
        || checked.backend.namespace_identity() != target.backend.namespace_identity() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "capacity export changed"));
    }
    Ok(value)
}
