//! Unix stage guards and permissions, exercised by the shared remote suite.
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, PermissionsExt};

use crate::local_access::NotRegular;
use crate::vfs::{
    finish_stage, promote_staged_create, Backend, LocalBackend, StageDurability, StageFinish,
};

#[test]
fn review_task_finish_stage_refuses_links_and_specials() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().to_str().unwrap();
    let backend = LocalBackend::new(root);
    let victim = fixture.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    let before = std::fs::metadata(&victim).unwrap().modified().unwrap();
    let stage = fixture.path().join("stage");
    symlink(&victim, &stage).unwrap();
    let finish = StageFinish {
        mtime_ms: Some(1_600_000_000_000),
        mode: Some(0o777),
        durability: StageDurability::Now,
    };
    let error = finish_stage(&backend, stage.to_str().unwrap(), finish).unwrap_err();
    assert_eq!(NotRegular::of(&error), Some(NotRegular::Link));
    assert_eq!(std::fs::metadata(&victim).unwrap().modified().unwrap(), before);
    std::fs::remove_file(&stage).unwrap();
    let fifo = CString::new(stage.as_os_str().as_bytes()).unwrap();
    // SAFETY: NUL-terminated fixture path and plain permission bits.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let error = finish_stage(&backend, stage.to_str().unwrap(), finish).unwrap_err();
    assert_eq!(NotRegular::of(&error), Some(NotRegular::Special));
    let destination = fixture.path().join("published");
    assert!(promote_staged_create(
        &backend,
        stage.to_str().unwrap(),
        destination.to_str().unwrap(),
    ).is_err());
    assert!(!destination.exists());
    assert!(std::fs::symlink_metadata(&stage).is_ok(), "refusal retains the stage");
}

#[test]
fn review_task_local_copy_preserves_private_destination_and_masks_setid() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().to_str().unwrap();
    let backend = LocalBackend::new(root);
    let source = fixture.path().join("source");
    let target = fixture.path().join("target");
    std::fs::write(&source, b"new").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o6754)).unwrap();
    std::fs::write(&target, b"old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        backend
            .copy_file(source.to_str().unwrap(), target.to_str().unwrap())
            .unwrap(),
        3,
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o640,
    );
}

#[test]
fn review_task_mkdir_rechecks_a_previously_created_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("root");
    let outside = fixture.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let backend = LocalBackend::new(root.to_str().unwrap());
    let previously_checked = root.join("checked");
    backend
        .mkdir_all(previously_checked.join("first").to_str().unwrap())
        .unwrap();
    std::fs::rename(&previously_checked, root.join("moved")).unwrap();
    symlink(&outside, &previously_checked).unwrap();
    assert!(backend
        .mkdir_all(previously_checked.join("second").to_str().unwrap())
        .is_err());
    assert!(!outside.join("second").exists());
}

#[test]
fn review_task_stage_finish_opens_a_read_only_unix_stage() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().to_str().unwrap();
    let stage = fixture.path().join("read-only");
    std::fs::write(&stage, b"complete").unwrap();
    std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o400)).unwrap();
    let finished = finish_stage(
        &LocalBackend::new(root),
        stage.to_str().unwrap(),
        StageFinish {
            mtime_ms: Some(1_600_000_000_123),
            mode: Some(0o400),
            durability: StageDurability::Now,
        },
    )
    .unwrap();
    assert!(finished.mtime_applied && finished.durable);
    assert_eq!(
        std::fs::metadata(stage).unwrap().permissions().mode() & 0o777,
        0o400,
    );
}
