use super::{
    export_config::ExportAccess,
    framing::{reply, reply_err},
    fs::ShareExportConfig,
    mount_lease::PeerMountLeases,
    session::PeerPrincipal,
    wire::{
        FsHostFeatures, FsResponse, FsTransferCapabilities, FsWriteCapabilities,
        MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
    },
};
use crate::vfs::StagedWriteCapabilities;
use iroh::endpoint::SendStream;
use std::{
    io,
    sync::{Arc, Mutex},
};

/// One Capabilities request of an authorized stream.
pub(super) struct CapabilityQuery {
    pub(super) path: String,
    pub(super) acquire_lease: bool,
    pub(super) lease_request_id: Option<String>,
    pub(super) exports: ShareExportConfig,
    pub(super) principal: PeerPrincipal,
    pub(super) legacy_connection: usize,
    pub(super) authorization_epoch: u64,
    pub(super) mount_leases: Arc<PeerMountLeases>,
    /// Host-wide transfer features, reported for every path (also `/`).
    pub(super) transfer: FsTransferCapabilities,
}

pub(super) async fn handle_capabilities(
    send: &mut SendStream,
    query: CapabilityQuery,
) -> io::Result<()> {
    let result = super::blocking::run("Share filesystem capabilities", move || {
        resolve_capabilities(query)
    })
    .await;
    match result {
        Ok(response) => reply(send, response).await,
        Err(error) => reply_err(send, error).await,
    }
}

fn resolve_capabilities(query: CapabilityQuery) -> io::Result<FsResponse> {
    let CapabilityQuery {
        path,
        acquire_lease,
        lease_request_id,
        exports,
        principal,
        legacy_connection,
        authorization_epoch,
        mount_leases,
        transfer,
    } = query;
    if acquire_lease {
        if let Some(grant) = mount_leases.existing_acquisition(
            &path,
            &exports,
            &principal,
            lease_request_id.as_deref(),
            legacy_connection,
            authorization_epoch,
        )? {
            let capabilities = grant.lease.capabilities();
            return Ok(describe(
                capabilities.staged_write,
                capabilities.root_confinement.is_enforced(),
                Some(grant.token),
                transfer,
                None,
            ));
        }
    }
    let snapshot = Arc::new(Mutex::new(exports.clone()));
    let resolved = super::fs_capabilities::resolve_mount_capabilities(&path, &snapshot)?;
    let Some(resolved) = resolved else {
        return Ok(describe(
            StagedWriteCapabilities::default(),
            false,
            None,
            transfer,
            None,
        ));
    };
    let access = Some(resolved.target.access);
    if !acquire_lease {
        let root_confined = resolved.lease_root_confined();
        return Ok(describe(
            resolved.capabilities.staged_write,
            root_confined,
            None,
            transfer,
            access,
        ));
    }
    let grant = mount_leases.acquire(
        resolved,
        exports,
        principal,
        lease_request_id,
        legacy_connection,
        authorization_epoch,
    )?;
    let capabilities = grant.lease.capabilities();
    Ok(describe(
        capabilities.staged_write,
        capabilities.root_confinement.is_enforced(),
        Some(grant.token),
        transfer,
        access,
    ))
}

/// `access`: of the export holding the path, where it was resolved here
/// (an existing lease's retry and the synthetic containers report none).
fn describe(
    staged_write: StagedWriteCapabilities,
    root_confined: bool,
    lease: Option<String>,
    transfer: FsTransferCapabilities,
    access: Option<ExportAccess>,
) -> FsResponse {
    let mut capabilities = FsWriteCapabilities::from(staged_write);
    capabilities.transfer = transfer;
    capabilities.features = FsHostFeatures::host();
    capabilities.access = access;
    FsResponse::Capabilities {
        capabilities,
        contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
        root_confined,
        lease,
        storage_snapshot_v1: true,
        storage_analysis_v2: true,
    }
}
