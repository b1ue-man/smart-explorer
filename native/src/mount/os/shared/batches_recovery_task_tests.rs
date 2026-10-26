//! B1: real offline package I/O in disposable roots; no provider connection.
use super::*;
use crate::mount::recovery_manifest::RecoveryGapKind;
use crate::mount::{BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountSource};

struct Fixture {
    _temp: tempfile::TempDir,
    base: PathBuf,
    exports: PathBuf,
    imports: PathBuf,
    restore: PathBuf,
    config: MountConfig,
}
impl Fixture {
    fn new(sealed: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("original");
        let exports = temp.path().join("exports");
        let imports = temp.path().join("imports");
        let restore = temp.path().join("restore");
        for path in [&base, &exports, &imports, &restore] {
            std::fs::create_dir(path).unwrap();
        }
        let source = if sealed {
            MountSource::Vault {
                site_id: "fixture-site".into(),
            }
        } else {
            MountSource::SavedRemote {
                account: "sftp-fixture".into(),
                root: BackendRoot::parse("/literal-root").unwrap(),
            }
        };
        let config = MountConfig::new(
            MountId::parse("fixture-mount").unwrap(),
            source,
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Offline fixture",
        )
        .unwrap();
        let cache = base.join(config.id.as_str());
        std::fs::create_dir_all(cache.join("files")).unwrap();
        std::fs::write(cache.join("files/payload"), b"unpublished\0local\xffbytes").unwrap();
        // Preserve uninterpretable state and torn tails instead of opening a spool.
        std::fs::write(cache.join("journal.jsonl"), b"torn journal without newline").unwrap();
        std::fs::write(
            cache.join("stages.json"),
            br#"{"stages":{"/remote-stage":{"pending":true}}}"#,
        )
        .unwrap();
        Self {
            _temp: temp,
            base,
            exports,
            imports,
            restore,
            config,
        }
    }
    fn export(&self) -> (PathBuf, RecoveryManifest, String) {
        crate::mount::export_mount_recovery(
            &self.base,
            &self.config,
            &[self.config.id.clone()],
            &self.exports,
            serde_json::to_value(&self.config.source).unwrap(),
        )
        .unwrap()
    }
    fn record(
        &self,
        path: PathBuf,
        manifest: &RecoveryManifest,
        digest: String,
    ) -> InactiveMountRecovery {
        InactiveMountRecovery {
            id: manifest.recovery_id.clone(),
            config: manifest.config.clone(),
            retained_cache: path.join("cache"),
            package_path: path,
            manifest_sha256: digest,
            cache_ids: vec![self.config.id.clone()],
            reactivated: false,
            owns_mount_cache: false,
            sealed: manifest.sealed,
            local_snapshot_complete: manifest.local_snapshot_complete,
            gap_count: manifest.gaps.len() as u64,
            gaps: manifest.gaps.clone(),
            package_error: None,
        }
    }
}

#[test]
fn batches_task_recovery_offline_export_import_preserves_raw_bytes_after_source_removal() {
    let fixture = Fixture::new(false);
    let (package, manifest, digest) = fixture.export();
    assert!(manifest.local_snapshot_complete);
    assert!(manifest
        .gaps
        .iter()
        .any(|gap| gap.kind == RecoveryGapKind::TornJournalTail));
    assert!(manifest
        .gaps
        .iter()
        .any(|gap| gap.kind == RecoveryGapKind::RemoteStageNotBackedUp));
    assert_eq!(manifest.files.len(), 3);
    let original = fixture.base.join(fixture.config.id.as_str());
    for file in &manifest.files {
        let relative: PathBuf = file.components.iter().collect();
        assert_eq!(
            std::fs::read(fixture.base.join(&relative)).unwrap(),
            std::fs::read(package.join("cache").join(relative)).unwrap()
        );
    }
    let (owned, imported, copied_digest) =
        copy_recovery_package(&package, &fixture.imports).unwrap();
    assert_eq!(digest, copied_digest);
    assert_eq!(manifest, imported);
    std::fs::remove_dir_all(&package).unwrap();
    verify_recovery_package(&owned, Some(&digest)).unwrap();
    let record = fixture.record(owned, &imported, copied_digest);
    restore_recovery_cache(&record, &fixture.restore).unwrap();
    restore_recovery_cache(&record, &fixture.restore).unwrap();
    assert_eq!(
        std::fs::read(original.join("files/payload")).unwrap(),
        std::fs::read(
            fixture
                .restore
                .join(fixture.config.id.as_str())
                .join("files/payload")
        )
        .unwrap()
    );
    assert!(!record.owns_mount_cache);
    assert!(!record.reactivated);
}

#[test]
fn batches_task_recovery_tamper_and_conflicting_cache_preserve_originals() {
    let fixture = Fixture::new(false);
    let (package, manifest, digest) = fixture.export();
    let record = fixture.record(package.clone(), &manifest, digest.clone());
    let occupied = fixture.restore.join(fixture.config.id.as_str());
    std::fs::create_dir_all(occupied.join("files")).unwrap();
    std::fs::write(occupied.join("files/payload"), b"existing user bytes").unwrap();
    assert_eq!(
        restore_recovery_cache(&record, &fixture.restore)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        std::fs::read(occupied.join("files/payload")).unwrap(),
        b"existing user bytes"
    );
    verify_recovery_package(&package, Some(&digest)).unwrap();
    assert!(verify_recovery_package(&package, Some(&"0".repeat(64))).is_err());
    std::fs::write(
        package
            .join("cache")
            .join(fixture.config.id.as_str())
            .join("files/payload"),
        b"tampered",
    )
    .unwrap();
    assert!(verify_recovery_package(&package, Some(&digest)).is_err());
    assert!(copy_recovery_package(&package, &fixture.imports).is_err());
    assert_eq!(std::fs::read_dir(&fixture.imports).unwrap().count(), 0);
    assert_eq!(
        std::fs::read(
            fixture
                .base
                .join(fixture.config.id.as_str())
                .join("files/payload")
        )
        .unwrap(),
        b"unpublished\0local\xffbytes"
    );
}

#[test]
fn batches_task_recovery_sealed_bytes_report_unknown_inventory_without_invented_entries() {
    let fixture = Fixture::new(true);
    let (package, manifest, digest) = fixture.export();
    assert!(manifest.sealed);
    assert!(manifest.local_snapshot_complete);
    assert!(manifest
        .gaps
        .iter()
        .any(|gap| gap.kind == RecoveryGapKind::MissingVaultKey));
    assert!(manifest
        .gaps
        .iter()
        .any(|gap| gap.kind == RecoveryGapKind::JournalInventoryUnknown));
    assert!(!manifest.gaps.iter().any(|gap| matches!(
        gap.kind,
        RecoveryGapKind::MissingPayload | RecoveryGapKind::RemoteStageNotBackedUp
    )));
    assert!(manifest
        .journals
        .iter()
        .all(|journal| journal.entries.is_empty()
            && journal.deletes.is_empty()
            && journal.namespace_intents.is_empty()));
    verify_recovery_package(&package, Some(&digest)).unwrap();
    let (owned, _, _) = copy_recovery_package(&package, &fixture.imports).unwrap();
    assert_eq!(
        std::fs::read(
            owned
                .join("cache")
                .join(fixture.config.id.as_str())
                .join("journal.jsonl")
        )
        .unwrap(),
        b"torn journal without newline"
    );
}

#[cfg(unix)]
#[test]
fn batches_task_recovery_export_refuses_symlink_without_reading_external_payload() {
    let fixture = Fixture::new(false);
    let outside = fixture.imports.join("outside");
    std::fs::write(&outside, b"outside-authorized-cache").unwrap();
    std::os::unix::fs::symlink(
        &outside,
        fixture
            .base
            .join(fixture.config.id.as_str())
            .join("files/link"),
    )
    .unwrap();
    assert!(crate::mount::export_mount_recovery(
        &fixture.base,
        &fixture.config,
        &[fixture.config.id.clone()],
        &fixture.exports,
        serde_json::to_value(&fixture.config.source).unwrap()
    )
    .is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"outside-authorized-cache");
}
