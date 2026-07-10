use super::*;
use crate::hash::sha256_file;

#[test]
fn atomic_existing_boundary_keeps_target_populated() {
    let dir = tempfile::tempdir().unwrap();
    let pending = dir.path().join("pending");
    let target = dir.path().join("target");
    let backup = dir.path().join("backup");
    std::fs::write(&pending, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();

    let result =
        replace_platform::replace_existing_with_guard(&pending, &target, &backup, |backup_ready| {
            assert!(target.exists());
            assert_eq!(std::fs::read(&target).unwrap(), b"old");
            if backup_ready {
                assert_eq!(std::fs::read(&backup).unwrap(), b"old");
            }
            Ok::<(), String>(())
        });

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(std::fs::read(target).unwrap(), b"new");
    assert_eq!(std::fs::read(backup).unwrap(), b"old");
}

#[test]
fn transaction_rejects_tamper_without_changing_any_target() {
    let dir = tempfile::tempdir().unwrap();
    let app_staged = dir.path().join("app-staged");
    let cli_staged = dir.path().join("cli-staged");
    let app_target = dir.path().join("app");
    let cli_target = dir.path().join("cli");
    std::fs::write(&app_staged, b"new-app").unwrap();
    std::fs::write(&cli_staged, b"new-cli").unwrap();
    std::fs::write(&app_target, b"old-app").unwrap();
    std::fs::write(&cli_target, b"old-cli").unwrap();
    let app_hash = sha256_file(&app_staged).unwrap();
    let cli_hash = sha256_file(&cli_staged).unwrap();
    std::fs::write(&cli_staged, b"bad-cli").unwrap();

    let result = replace_transaction(&[
        Replacement {
            label: "App",
            staged: &app_staged,
            target: &app_target,
            sha256: &app_hash,
            expected_target_sha256: None,
        },
        Replacement {
            label: "CLI",
            staged: &cli_staged,
            target: &cli_target,
            sha256: &cli_hash,
            expected_target_sha256: None,
        },
    ]);

    assert!(result.is_err());
    assert_eq!(std::fs::read(&app_target).unwrap(), b"old-app");
    assert_eq!(std::fs::read(&cli_target).unwrap(), b"old-cli");
}

#[test]
fn explicit_rollback_restores_all_replaced_targets() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let hash = sha256_file(&staged).unwrap();
    let mut transaction = replace_transaction(&[Replacement {
        label: "App",
        staged: &staged,
        target: &target,
        sha256: &hash,
        expected_target_sha256: None,
    }])
    .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");

    transaction.rollback().unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"old");
}

#[test]
fn changed_guarded_target_is_rejected_before_rename() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let staged_hash = sha256_file(&staged).unwrap();
    let old_hash = sha256_file(&target).unwrap();
    std::fs::write(&target, b"drifted").unwrap();

    let result = replace_transaction(&[Replacement {
        label: "App",
        staged: &staged,
        target: &target,
        sha256: &staged_hash,
        expected_target_sha256: Some(&old_hash),
    }]);

    assert!(result.is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"drifted");
}

#[test]
fn drift_after_staging_is_rejected_and_original_target_path_is_restored() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let staged_hash = sha256_file(&staged).unwrap();
    let old_hash = sha256_file(&target).unwrap();

    let result = replace_transaction_impl(
        &[Replacement {
            label: "App",
            staged: &staged,
            target: &target,
            sha256: &staged_hash,
            expected_target_sha256: Some(&old_hash),
        }],
        || std::fs::write(&target, b"drifted").unwrap(),
    );

    assert!(result.is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"drifted");
}

#[test]
fn pending_tamper_after_copy_is_rejected_before_install() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let staged_hash = sha256_file(&staged).unwrap();
    let old_hash = sha256_file(&target).unwrap();

    let result = replace_transaction_impl(
        &[Replacement {
            label: "App",
            staged: &staged,
            target: &target,
            sha256: &staged_hash,
            expected_target_sha256: Some(&old_hash),
        }],
        || {
            let pending = std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .find(|path| path.to_string_lossy().contains("update-pending"))
                .unwrap();
            std::fs::write(pending, b"bad").unwrap();
        },
    );

    assert!(result.is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"old");
}

#[test]
fn later_pending_tamper_rolls_back_every_earlier_target() {
    let dir = tempfile::tempdir().unwrap();
    let app_staged = dir.path().join("app-staged");
    let cli_staged = dir.path().join("cli-staged");
    let app_target = dir.path().join("app");
    let cli_target = dir.path().join("cli");
    std::fs::write(&app_staged, b"new-app").unwrap();
    std::fs::write(&cli_staged, b"new-cli").unwrap();
    std::fs::write(&app_target, b"old-app").unwrap();
    std::fs::write(&cli_target, b"old-cli").unwrap();
    let app_hash = sha256_file(&app_staged).unwrap();
    let cli_hash = sha256_file(&cli_staged).unwrap();

    let result = replace_transaction_impl(
        &[
            Replacement {
                label: "App",
                staged: &app_staged,
                target: &app_target,
                sha256: &app_hash,
                expected_target_sha256: None,
            },
            Replacement {
                label: "CLI",
                staged: &cli_staged,
                target: &cli_target,
                sha256: &cli_hash,
                expected_target_sha256: None,
            },
        ],
        || {
            let cli_pending = std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with("cli.update-pending")
                    })
                })
                .unwrap();
            std::fs::write(cli_pending, b"tampered-cli").unwrap();
        },
    );

    assert!(result.is_err());
    assert_eq!(std::fs::read(app_target).unwrap(), b"old-app");
    assert_eq!(std::fs::read(cli_target).unwrap(), b"old-cli");
}

#[test]
fn absent_target_race_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    let hash = sha256_file(&staged).unwrap();

    let result = replace_transaction_impl(
        &[Replacement {
            label: "App",
            staged: &staged,
            target: &target,
            sha256: &hash,
            expected_target_sha256: None,
        }],
        || std::fs::write(&target, b"racer").unwrap(),
    );

    assert!(result.is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"racer");
}

#[test]
fn missing_rollback_backup_does_not_delete_only_runnable_target() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let hash = sha256_file(&staged).unwrap();
    let mut transaction = replace_transaction(&[Replacement {
        label: "App",
        staged: &staged,
        target: &target,
        sha256: &hash,
        expected_target_sha256: None,
    }])
    .unwrap();
    let backup = transaction.prepared[0].old.clone();
    std::fs::remove_file(backup).unwrap();

    assert!(transaction.rollback().is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    transaction.finalize();
}

#[test]
fn partial_replace_state_with_missing_target_restores_verified_backup() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let hash = sha256_file(&staged).unwrap();
    let mut transaction = replace_transaction(&[Replacement {
        label: "App",
        staged: &staged,
        target: &target,
        sha256: &hash,
        expected_target_sha256: None,
    }])
    .unwrap();
    std::fs::remove_file(&target).unwrap();

    replace_platform::recover_failed_install(&transaction.prepared[0]).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    transaction.rollback().unwrap();
}

#[test]
fn missing_target_is_restored_before_corrupt_pending_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let hash = sha256_file(&staged).unwrap();
    let mut transaction = replace_transaction(&[Replacement {
        label: "App",
        staged: &staged,
        target: &target,
        sha256: &hash,
        expected_target_sha256: None,
    }])
    .unwrap();
    let pending = transaction.prepared[0].pending.clone();
    std::fs::remove_file(&target).unwrap();
    std::fs::write(&pending, b"corrupt-pending").unwrap();

    assert!(replace_platform::recover_failed_install(&transaction.prepared[0]).is_err());

    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    std::fs::remove_file(pending).unwrap();
    transaction.rollback().unwrap();
}

#[test]
fn immediate_post_install_corruption_restores_verified_backup() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let hash = sha256_file(&staged).unwrap();
    let mut transaction = replace_transaction(&[Replacement {
        label: "App",
        staged: &staged,
        target: &target,
        sha256: &hash,
        expected_target_sha256: None,
    }])
    .unwrap();
    transaction.prepared[0].post_install_invalid = true;
    std::fs::write(&target, b"corrupt-after-boundary").unwrap();

    transaction.rollback().unwrap();

    assert_eq!(std::fs::read(target).unwrap(), b"old");
}

#[test]
fn rollback_preflight_avoids_partial_restore_after_target_tamper() {
    let dir = tempfile::tempdir().unwrap();
    let bad_staged = dir.path().join("bad-staged");
    let good_staged = dir.path().join("good-staged");
    let bad_target = dir.path().join("bad-target");
    let good_target = dir.path().join("good-target");
    std::fs::write(&bad_staged, b"bad-new").unwrap();
    std::fs::write(&good_staged, b"good-new").unwrap();
    std::fs::write(&bad_target, b"bad-old").unwrap();
    std::fs::write(&good_target, b"good-old").unwrap();
    let bad_hash = sha256_file(&bad_staged).unwrap();
    let good_hash = sha256_file(&good_staged).unwrap();
    let mut transaction = replace_transaction(&[
        Replacement {
            label: "Bad",
            staged: &bad_staged,
            target: &bad_target,
            sha256: &bad_hash,
            expected_target_sha256: None,
        },
        Replacement {
            label: "Good",
            staged: &good_staged,
            target: &good_target,
            sha256: &good_hash,
            expected_target_sha256: None,
        },
    ])
    .unwrap();
    std::fs::write(&bad_target, b"concurrent-change").unwrap();

    assert!(transaction.rollback().is_err());
    assert_eq!(std::fs::read(&good_target).unwrap(), b"good-new");
    assert!(transaction.rollback().is_err());
    assert_eq!(std::fs::read(&good_target).unwrap(), b"good-new");

    std::fs::write(&bad_target, b"bad-new").unwrap();
    transaction.rollback().unwrap();
    assert_eq!(std::fs::read(bad_target).unwrap(), b"bad-old");
    assert_eq!(std::fs::read(good_target).unwrap(), b"good-old");
}

#[test]
fn finalize_surfaces_backup_cleanup_failures() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let hash = sha256_file(&staged).unwrap();
    let mut transaction = replace_transaction(&[Replacement {
        label: "App",
        staged: &staged,
        target: &target,
        sha256: &hash,
        expected_target_sha256: None,
    }])
    .unwrap();
    let backup = transaction.prepared[0].old.clone();
    std::fs::remove_file(&backup).unwrap();
    std::fs::create_dir(&backup).unwrap();

    let warnings = transaction.finish_cleanup();

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Rollback-Sicherung"));
    assert_eq!(std::fs::read(target).unwrap(), b"new");
}
