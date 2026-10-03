//! Windows stages copied from read-only sources must still be finishable.
use crate::vfs::{finish_stage, LocalBackend, StageDurability, StageFinish};

#[test]
fn review_task_windows_read_only_stage_keeps_its_attribute_when_finished() {
    let fixture = tempfile::tempdir().unwrap();
    let stage = fixture.path().join("stage");
    std::fs::write(&stage, b"complete").unwrap();
    let mut permissions = std::fs::metadata(&stage).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&stage, permissions).unwrap();
    let root = fixture.path().to_string_lossy().replace('\\', "/");
    let stage_vfs = stage.to_string_lossy().replace('\\', "/");
    let result = finish_stage(
        &LocalBackend::new(&root),
        &stage_vfs,
        StageFinish {
            mtime_ms: Some(1_600_000_000_123),
            mode: Some(0o600),
            durability: StageDurability::Now,
        },
    );
    let mut permissions = std::fs::metadata(&stage).unwrap().permissions();
    let remained_read_only = permissions.readonly();
    permissions.set_readonly(false);
    std::fs::set_permissions(&stage, permissions).unwrap();
    let finished = result.unwrap();
    assert!(finished.mtime_applied && finished.durable);
    assert!(remained_read_only);
}

#[test]
fn review_task_windows_new_names_validate_every_component() {
    for relative in ["parent:stream/child", "bad?/child", "control\u{1}/child"] {
        let path = std::path::Path::new("C:/chosen").join(relative);
        let error = super::check_new_name(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidFilename);
    }
    // Stored names remain addressable using the existing verbatim path API;
    // the planner, rather than an OS writer, omits newly imported ones.
    assert!(super::check_new_name(std::path::Path::new(r"C:\chosen\NUL.txt")).is_ok());
    assert!(super::check_new_name(std::path::Path::new(r"C:\chosen\trailing.")).is_ok());
}
