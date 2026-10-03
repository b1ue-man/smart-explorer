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
use crate::vfs::{StagedWriteCapabilities, TargetLimits};
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
    pub(super) may_write: bool,
    pub(super) access: super::fs_access::FsAccess,
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
    let principal = query.principal.clone();
    let authority = query.access.clone();
    let _transport_guard = authority.stream_guard();
    let stopped = send.stopped();
    let result = tokio::select! {
        result = super::blocking::run_for(principal, super::blocking::Class::Control, "Share filesystem capabilities", move || {
            resolve_capabilities(query)
        }) => result,
        _ = stopped => return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Share-Anfrage beendet")),
    };
    authority.check_read()?;
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
        exports: _,
        may_write,
        access: authority,
        principal,
        legacy_connection,
        authorization_epoch,
        mount_leases,
        transfer,
    } = query;
    authority.check_read()?;
    let exports = authority.export_snapshot()?;
    let may_write = may_write && authority.check_write().is_ok();
    if acquire_lease {
        if let Some(grant) = mount_leases.existing_acquisition(
            &path,
            &exports,
            &principal,
            lease_request_id.as_deref(),
            legacy_connection,
            authorization_epoch,
        )? {
            let mut capabilities = grant.lease.capabilities();
            if !may_write { capabilities.staged_write = StagedWriteCapabilities::default(); }
            let target = grant.lease.resolve(&path)?;
            let limits = crate::vfs::target_limits(&*target.backend, &target.path);
            return Ok(describe(
                capabilities.staged_write,
                capabilities.root_confinement.is_enforced(),
                Some(grant.token),
                transfer,
                Some(if may_write { target.access } else { ExportAccess::ReadOnly }),
                limits,
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
            TargetLimits::default(),
        ));
    };
    let mut resolved = resolved;
    if !may_write { resolved.capabilities.staged_write = StagedWriteCapabilities::default(); }
    let access = Some(if may_write { resolved.target.access } else { ExportAccess::ReadOnly });
    let limits = crate::vfs::target_limits(&*resolved.target.backend, &resolved.target.path);
    if !acquire_lease {
        let root_confined = resolved.lease_root_confined();
        return Ok(describe(
            resolved.capabilities.staged_write,
            root_confined,
            None,
            transfer,
            access,
            limits,
        ));
    }
    let grant = mount_leases.acquire(
        resolved,
        exports,
        principal.clone(),
        lease_request_id,
        legacy_connection,
        authorization_epoch,
    )?;
    if let Err(error) = authority.check_read() {
        let _ = mount_leases.release(&grant.token, &principal);
        return Err(error);
    }
    let capabilities = grant.lease.capabilities();
    Ok(describe(
        capabilities.staged_write,
        capabilities.root_confinement.is_enforced(),
        Some(grant.token),
        transfer,
        access,
        limits,
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
    limits: TargetLimits,
) -> FsResponse {
    let mut capabilities = FsWriteCapabilities::from(staged_write);
    capabilities.transfer = transfer;
    capabilities.features = FsHostFeatures::host();
    capabilities.access = access;
    capabilities.limits = limits.into();
    FsResponse::Capabilities {
        capabilities,
        contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
        root_confined,
        lease,
        storage_snapshot_v1: true,
        storage_analysis_v2: true,
    }
}
