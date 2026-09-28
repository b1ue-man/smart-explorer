use std::{io, sync::{Arc, Mutex}};
use iroh::endpoint::SendStream;
use super::{framing::{reply, reply_err}, fs::ShareExportConfig,
    mount_lease::PeerMountLeases, session::PeerPrincipal,
    wire::{FsResponse, MOUNT_PATH_CAPABILITY_CONTRACT_VERSION}};

pub(super) async fn handle_capabilities(
    send: &mut SendStream,
    path: String,
    acquire_lease: bool,
    exports: ShareExportConfig,
    principal: PeerPrincipal,
    lease_request_id: Option<String>,
    legacy_connection: usize,
    authorization_epoch: u64,
    mount_leases: Arc<PeerMountLeases>,
) -> io::Result<()> {
    let result = super::blocking::run("Share filesystem capabilities", move || {
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
                return Ok(FsResponse::Capabilities {
                    capabilities: capabilities.staged_write.into(),
                    contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                    root_confined: capabilities.root_confinement.is_enforced(),
                    lease: Some(grant.token),
                    storage_snapshot_v1: true,
                    storage_analysis_v2: true,
                });
            }
        }
        let snapshot = Arc::new(Mutex::new(exports.clone()));
        let resolved = super::fs_capabilities::resolve_mount_capabilities(&path, &snapshot)?;
        let Some(resolved) = resolved else {
            return Ok(FsResponse::Capabilities {
                capabilities: Default::default(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined: false,
                lease: None,
                storage_snapshot_v1: true,
                storage_analysis_v2: true,
            });
        };
        if !acquire_lease {
            let root_confined = resolved.lease_root_confined();
            return Ok(FsResponse::Capabilities {
                capabilities: resolved.capabilities.staged_write.into(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined,
                lease: None,
                storage_snapshot_v1: true,
                storage_analysis_v2: true,
            });
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
        Ok(FsResponse::Capabilities {
            capabilities: capabilities.staged_write.into(),
            contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
            root_confined: capabilities.root_confinement.is_enforced(),
            lease: Some(grant.token),
            storage_snapshot_v1: true,
            storage_analysis_v2: true,
        })
    })
    .await;
    match result {
        Ok(response) => reply(send, response).await,
        Err(error) => reply_err(send, error).await,
    }
}

