use super::fs::{ShareExportConfig, SharedRoot};
use super::fs_capabilities::{resolve_mount_capabilities, ResolvedMountCapabilities};
use super::mount_lease::{PeerMountLeases, RELEASABLE_LEASE_PREFIX};
use super::mount_lease_client::PeerMountLeaseClient;
use super::session::PeerPrincipal;
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
    assert_eq!(client.take_releasable().unwrap(), None);
    assert_eq!(client.current().unwrap(), None);

    let releasable_token = format!("{RELEASABLE_LEASE_PREFIX}lease-current");
    client
        .accept_capabilities(
            FsResponse::Capabilities {
                capabilities: full_wire_capabilities(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined: true,
                lease: Some(releasable_token.clone()),
            },
            true,
        )
        .unwrap();
    assert_eq!(
        client.take_releasable().unwrap().as_deref(),
        Some(releasable_token.as_str())
    );
    assert_eq!(client.current().unwrap(), None);
}

#[test]
fn remote_drive_task_peer_mount_lease_is_principal_root_config_and_epoch_bound() -> io::Result<()> {
    let fixture = LocalLeaseFixture::new()?;
    let leases = PeerMountLeases::default();
    let principal = principal("device-a");
    let first = leases.acquire(
        fixture.resolved()?,
        fixture.exports.clone(),
        principal.clone(),
        Some("mount-a".into()),
        100,
        7,
    )?;
    assert_eq!(
        first.lease.capabilities().root_confinement,
        RootConfinement::Unverified
    );
    let retry = leases
        .existing_acquisition(
            "/Docs",
            &fixture.exports,
            &principal,
            Some("mount-a"),
            100,
            7,
        )?
        .expect("safe capability retry must reuse its acquisition");
    assert_eq!(retry.token, first.token);
    assert!(Arc::ptr_eq(&retry.lease, &first.lease));

    let duplicate = leases.acquire(
        fixture.resolved()?,
        fixture.exports.clone(),
        principal.clone(),
        Some("mount-a".into()),
        100,
        7,
    )?;
    assert_eq!(duplicate.token, first.token);
    assert!(Arc::ptr_eq(&duplicate.lease, &first.lease));

    let renewed = leases.acquire(
        fixture.resolved()?,
        fixture.exports.clone(),
        principal.clone(),
        Some("mount-a".into()),
        200,
        8,
    )?;
    assert_ne!(renewed.token, first.token);
    assert_eq!(
        only_error(leases.authorize(&first.token, &principal, &fixture.exports, 200, 8)).kind(),
        io::ErrorKind::PermissionDenied
    );
    leases.authorize(&renewed.token, &principal, &fixture.exports, 999, 8)?;

    let other_principal = principal("device-b");
    assert_eq!(
        only_error(leases.authorize(&renewed.token, &other_principal, &fixture.exports, 999, 8,))
            .kind(),
        io::ErrorKind::PermissionDenied
    );

    let mut changed_exports = fixture.exports.clone();
    changed_exports.include_connections = true;
    assert_eq!(
        only_error(leases.authorize(&renewed.token, &principal, &changed_exports, 999, 8)).kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        only_error(renewed.lease.resolve("/Other/file.txt")).kind(),
        io::ErrorKind::PermissionDenied
    );
    let parallel_mount = leases.acquire(
        fixture.resolved()?,
        fixture.exports.clone(),
        principal.clone(),
        Some("mount-b".into()),
        999,
        8,
    )?;
    assert_ne!(parallel_mount.token, renewed.token);
    assert!(leases.release(&renewed.token, &principal)?.is_some());
    leases.authorize(&parallel_mount.token, &principal, &fixture.exports, 321, 8)?;
    leases.clear()?;
    assert_eq!(
        only_error(leases.authorize(&renewed.token, &principal, &fixture.exports, 999, 8)).kind(),
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
fn remote_drive_task_legacy_lease_stays_connection_scoped() -> io::Result<()> {
    let fixture = LocalLeaseFixture::new()?;
    let leases = PeerMountLeases::default();
    let principal = principal("legacy-device");
    let lease = leases.acquire(
        fixture.resolved()?,
        fixture.exports.clone(),
        principal.clone(),
        None,
        41,
        3,
    )?;
    let token = lease.token.clone();
    drop(lease);
    leases.authorize(&token, &principal, &fixture.exports, 41, 3)?;
    assert_eq!(
        only_error(leases.authorize(&token, &principal, &fixture.exports, 42, 3)).kind(),
        io::ErrorKind::PermissionDenied
    );
    let removed = leases.take_legacy_connection(41)?;
    assert_eq!(removed.len(), 1);
    assert_eq!(
        only_error(leases.authorize(&token, &principal, &fixture.exports, 41, 3)).kind(),
        io::ErrorKind::PermissionDenied
    );
    // The server hands this vector to its bounded blocking disposer, so a
    // runtime-owning backend cannot be finalized under the table lock/Iroh task.
    drop(removed);
    Ok(())
}

#[test]
fn remote_drive_task_lease_resources_have_a_small_recoverable_principal_bound() -> io::Result<()> {
    let fixture = LocalLeaseFixture::new()?;
    let leases = PeerMountLeases::default();
    let principal = principal("bounded-device");
    let mut tokens = Vec::new();
    for index in 0..4 {
        tokens.push(
            leases
                .acquire(
                    fixture.resolved()?,
                    fixture.exports.clone(),
                    principal.clone(),
                    Some(format!("mount-{index}")),
                    9,
                    1,
                )?
                .token,
        );
    }
    let overflow = leases.acquire(
        fixture.resolved()?,
        fixture.exports.clone(),
        principal.clone(),
        Some("mount-overflow".into()),
        9,
        1,
    );
    assert_eq!(only_error(overflow).kind(), io::ErrorKind::WouldBlock);
    assert!(leases.release(&tokens[0], &principal)?.is_some());
    leases.acquire(
        fixture.resolved()?,
        fixture.exports.clone(),
        principal,
        Some("mount-recovered".into()),
        9,
        1,
    )?;
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

fn principal(device: &str) -> PeerPrincipal {
    PeerPrincipal::new(
        "direct",
        "lookup",
        device,
        format!("key-{device}"),
        format!("node-{device}"),
    )
}
