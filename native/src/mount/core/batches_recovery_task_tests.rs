//! B1: offline manifest acceptance and historical principal boundaries.
use super::*;
use crate::mount::recovery_identity::{manifest_identity, RecoveryBackendIdentity};
use crate::mount::recovery_manifest::{RecoveryFile, RecoveryGap, RecoveryGapKind};
use crate::mount::{BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountSource};

fn manifest() -> RecoveryManifest {
    let source = MountSource::GoogleDrive {
        account: "cloud:gdrive".into(),
        root: BackendRoot::parse("/folder-id").unwrap(),
    };
    RecoveryManifest {
        version: RECOVERY_FORMAT_VERSION,
        import_version: RECOVERY_FORMAT_VERSION,
        recovery_id: "recovery-fixture".into(),
        created_unix_ms: 1,
        config: MountConfig::new(
            MountId::parse("mount-fixture").unwrap(),
            source.clone(),
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Fixture",
        )
        .unwrap(),
        backend_identity: serde_json::to_value(source).unwrap(),
        sealed: false,
        files: vec![],
        directories: vec![vec!["mount-fixture".into()]],
        journals: vec![],
        gaps: vec![],
        local_snapshot_complete: true,
        retained_original_cache: "/retained".into(),
    }
}

#[test]
fn batches_task_recovery_legacy_source_is_readable_but_not_principal_authority() {
    let mut package = manifest();
    validate_manifest(&package).unwrap();
    assert!(
        manifest_identity(&package.backend_identity, &package.config.source)
            .unwrap()
            .is_none()
    );
    let principal = "a".repeat(64);
    let identity = RecoveryBackendIdentity {
        version: 1,
        source: package.config.source.clone(),
        provider: "gdrive".into(),
        namespace: format!("gdrive:path-v2:{principal}"),
        state_identity: format!("gdrive:path-v2:{principal}:session"),
        principal,
        root_object: "immutable-folder-id".into(),
    };
    package.backend_identity = serde_json::to_value(&identity).unwrap();
    validate_manifest(&package).unwrap();
    assert_eq!(
        manifest_identity(&package.backend_identity, &package.config.source).unwrap(),
        Some(identity.clone())
    );
    identity
        .matches_session(&identity.state_identity, &identity.namespace)
        .unwrap();
    assert!(identity
        .matches_session("replacement-session", &identity.namespace)
        .is_err());
    assert!(identity
        .matches_session(&identity.state_identity, "replacement-account")
        .is_err());
    for field in ["principal", "namespace", "root_object"] {
        let mut changed = package.clone();
        changed.backend_identity[field] = serde_json::json!(if field == "root_object" {
            "root"
        } else {
            "wrong"
        });
        assert!(validate_manifest(&changed).is_err(), "{field}");
    }
}

#[test]
fn batches_task_recovery_manifest_rejects_incomplete_inventory_and_unsafe_names() {
    let good = manifest();
    let mut bad = good.clone();
    bad.local_snapshot_complete = false;
    assert!(validate_manifest(&bad).is_err());
    bad = good.clone();
    bad.directories.clear();
    assert!(validate_manifest(&bad).is_err());
    bad.gaps.push(RecoveryGap {
        kind: RecoveryGapKind::MissingCache,
        subject: "mount-fixture".into(),
        detail: "Explicitly absent".into(),
    });
    validate_manifest(&bad).unwrap();
    for name in ["..", ".", "", "x/y", "x\\y", "x:y", "x\0y"] {
        bad = good.clone();
        bad.files.push(RecoveryFile {
            components: vec!["mount-fixture".into(), name.into()],
            length: 1,
            sha256: "a".repeat(64),
        });
        assert!(validate_manifest(&bad).is_err(), "{name:?}");
    }
    bad = good.clone();
    bad.directories.push(vec!["mount-fixture".into()]);
    assert!(validate_manifest(&bad).is_err());
    bad = good.clone();
    bad.sealed = true;
    assert!(validate_manifest(&bad).is_err());
    bad = good;
    bad.import_version += 1;
    assert_eq!(
        validate_manifest(&bad).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
}
