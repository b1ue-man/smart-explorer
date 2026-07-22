use super::fs::{ShareExportConfig, SharedRoot};
use super::fs_capabilities::{resolve_mount_capabilities, ResolvedMountCapabilities};
use super::mount_lease::PeerMountLeases;
use super::mount_lease_client::PeerMountLeaseClient;
use super::wire::{FsResponse, FsWriteCapabilities, MOUNT_PATH_CAPABILITY_CONTRACT_VERSION};
use crate::vfs::{MountPathCapabilities, RootConfinement, StagedWriteCapabilities};
use std::io;
use std::sync::{Arc, Mutex};

struct LocalLeaseFixture {
    _temporary: tempfile::TempDir,
    exports: ShareExportConfig,
}

impl LocalLeaseFixture {
    fn new() -> io::Result<Self> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("shared");
        std::fs::create_dir(&root)?;
        Ok(Self {
            _temporary: temporary,
            exports: ShareExportConfig {
                roots: vec![SharedRoot {
                    label: "Docs".into(),
                    path: root.to_string_lossy().replace('\\', "/"),
                }],
                include_connections: false,
            },
        })
    }

    fn resolved(&self) -> io::Result<ResolvedMountCapabilities> {
        resolve_mount_capabilities("/Docs", &Arc::new(Mutex::new(self.exports.clone())))?
            .ok_or_else(|| io::Error::other("concrete root returned no capabilities"))
    }
}

fn full_wire_capabilities() -> FsWriteCapabilities {
    FsWriteCapabilities::from(StagedWriteCapabilities::complete())
}

#[test]
fn remote_drive_task_peer_capability_contract_is_conservative_and_token_bound() {
    let client = PeerMountLeaseClient::default();

    let acquired = client
        .accept_capabilities(
            FsResponse::Capabilities {
                capabilities: full_wire_capabilities(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined: true,
                lease: Some("lease-a".into()),
            },
            true,
        )
        .unwrap();
    assert_eq!(acquired.staged_write, StagedWriteCapabilities::complete());
    assert_eq!(acquired.root_confinement, RootConfinement::Enforced);
    assert_eq!(client.current().unwrap().as_deref(), Some("lease-a"));

    let legacy: FsResponse = serde_json::from_str(
        r#"{"r":"capabilities","capabilities":{"create":true,"replace":true,"namespace_replace":true}}"#,
    )
    .unwrap();
    assert_eq!(
        client.accept_capabilities(legacy, true).unwrap(),
        MountPathCapabilities::default()
    );
    assert_eq!(client.current().unwrap(), None);

    let preview = client
        .accept_capabilities(
            FsResponse::Capabilities {
                capabilities: full_wire_capabilities(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined: true,
                lease: None,
            },
            false,
        )
        .unwrap();
    assert_eq!(preview.staged_write, StagedWriteCapabilities::complete());
    assert_eq!(preview.root_confinement, RootConfinement::Enforced);
    assert_eq!(client.current().unwrap(), None);

    let unexpected_preview_lease = client.accept_capabilities(
        FsResponse::Capabilities {
            capabilities: full_wire_capabilities(),
            contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
            root_confined: true,
            lease: Some("probe-must-not-allocate".into()),
        },
        false,
    );
    assert_eq!(
        only_error(unexpected_preview_lease).kind(),
        io::ErrorKind::InvalidData
    );

    let lease_free = client
        .accept_capabilities(
            FsResponse::Capabilities {
                capabilities: full_wire_capabilities(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined: false,
                lease: None,
            },
            true,
        )
        .unwrap();
    assert_eq!(lease_free, MountPathCapabilities::default());
    assert_eq!(client.current().unwrap(), None);

    let trusted = client
        .accept_capabilities(
            FsResponse::Capabilities {
                capabilities: full_wire_capabilities(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined: false,
                lease: Some("lease-unverified".into()),
            },
            true,
        )
        .unwrap();
    assert_eq!(trusted.root_confinement, RootConfinement::Unverified);
    assert_eq!(
        client.current().unwrap().as_deref(),
        Some("lease-unverified")
    );
}

#[test]
fn remote_drive_task_peer_mount_lease_is_connection_root_config_and_epoch_bound() -> io::Result<()>
{
    let fixture = LocalLeaseFixture::new()?;
    let leases = PeerMountLeases::default();
    let first = leases.acquire(fixture.resolved()?, fixture.exports.clone(), 7)?;
    assert_eq!(
        first.lease.capabilities().root_confinement,
        RootConfinement::Unverified
    );

    let duplicate = leases.acquire(fixture.resolved()?, fixture.exports.clone(), 7)?;
    assert_eq!(duplicate.token, first.token);
    assert!(Arc::ptr_eq(&duplicate.lease, &first.lease));

    let renewed = leases.acquire(fixture.resolved()?, fixture.exports.clone(), 8)?;
    assert_ne!(renewed.token, first.token);
    assert_eq!(
        only_error(leases.authorize(&first.token, &fixture.exports, 8)).kind(),
        io::ErrorKind::PermissionDenied
    );
    leases.authorize(&renewed.token, &fixture.exports, 8)?;

    let other_connection = PeerMountLeases::default();
    assert_eq!(
        only_error(other_connection.authorize(&renewed.token, &fixture.exports, 8)).kind(),
        io::ErrorKind::PermissionDenied
    );

    let mut changed_exports = fixture.exports.clone();
    changed_exports.include_connections = true;
    assert_eq!(
        only_error(leases.authorize(&renewed.token, &changed_exports, 8)).kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        only_error(renewed.lease.resolve("/Other/file.txt")).kind(),
        io::ErrorKind::PermissionDenied
    );
    Ok(())
}

#[test]
fn remote_drive_task_peer_synthetic_roots_are_lease_free_and_read_only() -> io::Result<()> {
    let exports = Arc::new(Mutex::new(ShareExportConfig::default()));
    for root in ["/", "/Verbindungen"] {
        assert!(resolve_mount_capabilities(root, &exports)?.is_none());
    }
    assert!(!MountPathCapabilities::default()
        .staged_write
        .supports_mounted_writes());
    Ok(())
}

#[test]
fn remote_drive_task_peer_reserves_the_connections_container_name() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("colliding-label");
    std::fs::create_dir(&root)?;
    let exports = Arc::new(Mutex::new(ShareExportConfig {
        roots: vec![SharedRoot {
            label: "Verbindungen".into(),
            path: root.to_string_lossy().replace('\\', "/"),
        }],
        include_connections: false,
    }));

    let listing = super::fs::list_dir("/", &exports)?;
    assert_eq!(
        listing
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Verbindungen (2)"]
    );
    let resolved = super::fs::resolve("/Verbindungen (2)", &exports)?;
    assert!(resolved.backend.stat(&resolved.path)?.is_dir);
    Ok(())
}

fn only_error<T>(result: io::Result<T>) -> io::Error {
    match result {
        Ok(_) => panic!("operation unexpectedly succeeded"),
        Err(error) => error,
    }
}
