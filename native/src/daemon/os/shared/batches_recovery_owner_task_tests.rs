//! B1: real ownership document commit and readers, without profile/autostart I/O.
use super::*;
use crate::mount::{BackendRoot, DriveSelection, MountMode, MountSource};

struct OwnerFixture {
    root: tempfile::TempDir,
    record: InactiveMountRecovery,
}
impl OwnerFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        let package = root.path().join("package");
        std::fs::create_dir(&cache).unwrap();
        std::fs::create_dir(&package).unwrap();
        std::fs::write(cache.join("payload"), b"original pending bytes").unwrap();
        std::fs::write(package.join("payload"), b"saved pending bytes").unwrap();
        let config = MountConfig::new(
            MountId::parse("owner-fixture").unwrap(),
            MountSource::SavedRemote {
                account: "sftp-fixture".into(),
                root: BackendRoot::parse("/root").unwrap(),
            },
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Fixture",
        )
        .unwrap();
        let record = InactiveMountRecovery {
            id: "recovery-owner-fixture".into(),
            cache_ids: vec![config.id.clone()],
            config,
            package_path: package,
            retained_cache: cache,
            manifest_sha256: "a".repeat(64),
            reactivated: false,
            owns_mount_cache: true,
            sealed: false,
            local_snapshot_complete: true,
            gap_count: 0,
            gaps: vec![],
            package_error: None,
        };
        Self { root, record }
    }
    fn active_document(&self) -> Value {
        serde_json::json!({"version":1,"mounts":[self.record.config]})
    }
    fn inactive_document(&self) -> Value {
        serde_json::json!({"version":1,"mounts":[],"inactive_recoveries":{"version":1,"records":[self.record]}})
    }
    fn assert_payloads(&self) {
        assert_eq!(
            std::fs::read(self.record.retained_cache.join("payload")).unwrap(),
            b"original pending bytes"
        );
        assert_eq!(
            std::fs::read(self.record.package_path.join("payload")).unwrap(),
            b"saved pending bytes"
        );
    }
}

#[test]
fn batches_task_recovery_owner_document_commit_keeps_registry_readers_consistent() {
    let fixture = OwnerFixture::new();
    let directory = fixture.root.path();
    write_document(directory, FILES[0], &fixture.active_document()).unwrap();
    assert_eq!(
        super::super::load_in(directory).unwrap(),
        [fixture.record.config.clone()]
    );
    assert!(!is_inactive_in(directory, &fixture.record.config.id).unwrap());
    write_document(directory, FILES[0], &fixture.inactive_document()).unwrap();
    assert!(super::super::load_in(directory).unwrap().is_empty());
    assert!(is_inactive_in(directory, &fixture.record.config.id).unwrap());
    assert_eq!(list_in(directory).unwrap(), [fixture.record.clone()]);
    assert!(super::super::upsert_in(directory, &fixture.record.config).is_err());
    let unchanged = read_document(directory, FILES[0]).unwrap();
    assert_eq!(unchanged["mounts"], serde_json::json!([]));
    fixture.assert_payloads();
}

#[test]
fn batches_task_recovery_owner_rejects_conflicting_reactivation_source_without_losing_custody() {
    let fixture = OwnerFixture::new();
    let directory = fixture.root.path();
    write_document(directory, FILES[0], &fixture.inactive_document()).unwrap();
    let before = std::fs::read(directory.join(FILES[0])).unwrap();
    let mut changed = fixture.record.config.clone();
    changed.source = MountSource::SavedRemote {
        account: "foreign-account".into(),
        root: BackendRoot::parse("/root").unwrap(),
    };
    assert!(file_for_reactivation(directory, &changed).is_err());
    assert_eq!(std::fs::read(directory.join(FILES[0])).unwrap(), before);
    assert!(is_inactive_in(directory, &fixture.record.config.id).unwrap());
    assert!(super::super::load_in(directory).unwrap().is_empty());
    fixture.assert_payloads();
}

#[test]
fn batches_task_recovery_owner_oversized_commit_preserves_previous_document_and_payloads() {
    let fixture = OwnerFixture::new();
    let directory = fixture.root.path();
    for original in [fixture.active_document(), fixture.inactive_document()] {
        write_document(directory, FILES[0], &original).unwrap();
        let before = std::fs::read(directory.join(FILES[0])).unwrap();
        let mut oversized = fixture.inactive_document();
        oversized["oversized_fixture"] =
            Value::String("x".repeat(super::super::MAX_REGISTRY_BYTES as usize));
        assert!(write_document(directory, FILES[0], &oversized).is_err());
        assert_eq!(std::fs::read(directory.join(FILES[0])).unwrap(), before);
        fixture.assert_payloads();
    }
}

struct RecoveryFault;
impl RecoveryFault {
    fn at(point: crate::mount::RecoveryCommitPoint) -> Self {
        crate::mount::fail_next_recovery_at(Some(point));
        Self
    }
}
impl Drop for RecoveryFault {
    fn drop(&mut self) {
        crate::mount::fail_next_recovery_at(None);
    }
}

#[test]
fn batches_task_recovery_owner_injected_commit_failures_restore_active_or_inactive_origin() {
    use crate::mount::RecoveryCommitPoint::{OwnershipPublished, OwnershipSynced};
    for point in [OwnershipSynced, OwnershipPublished] {
        for reactivating in [false, true] {
            let fixture = OwnerFixture::new();
            let directory = fixture.root.path();
            let inactive = fixture.inactive_document();
            let active = fixture.active_document();
            let mut reactivated = inactive.clone();
            reactivated["mounts"] = serde_json::json!([fixture.record.config]);
            reactivated[FIELD]["records"][0]["owns_mount_cache"] = Value::Bool(false);
            reactivated[FIELD]["records"][0]["reactivated"] = Value::Bool(true);
            let (original, next) = if reactivating {
                (&inactive, &reactivated)
            } else {
                (&active, &inactive)
            };
            write_document(directory, FILES[0], original).unwrap();
            let before = std::fs::read(directory.join(FILES[0])).unwrap();
            let fault = RecoveryFault::at(point);
            let error = write_document(directory, FILES[0], next).unwrap_err();
            drop(fault);
            assert!(
                error
                    .to_string()
                    .contains("Injected recovery boundary failure"),
                "{point:?}: {error}"
            );
            assert_eq!(
                std::fs::read(directory.join(FILES[0])).unwrap(),
                before,
                "{point:?}, reactivating={reactivating}"
            );
            assert_eq!(
                is_inactive_in(directory, &fixture.record.config.id).unwrap(),
                reactivating
            );
            assert_eq!(
                super::super::load_in(directory).unwrap().is_empty(),
                reactivating
            );
            fixture.assert_payloads();
            // The same candidate remains retryable after the consumed one-shot fault.
            write_document(directory, FILES[0], next).unwrap();
            assert_eq!(
                is_inactive_in(directory, &fixture.record.config.id).unwrap(),
                !reactivating
            );
            assert_eq!(
                super::super::load_in(directory).unwrap().is_empty(),
                !reactivating
            );
            let records = list_in(directory).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].owns_mount_cache, !reactivating);
            assert_eq!(records[0].reactivated, reactivating);
            fixture.assert_payloads();
        }
    }
}
