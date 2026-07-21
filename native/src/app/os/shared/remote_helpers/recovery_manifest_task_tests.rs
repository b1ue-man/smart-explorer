use super::{preserve_marker_state, PreserveMarkerState, LEGACY_MANIFEST_HEADER, PRESERVE_MARKER};

fn write_marker(directory: &std::path::Path, contents: &str) {
    std::fs::write(directory.join(PRESERVE_MARKER), contents).unwrap();
}

#[test]
fn remote_drive_task_empty_complete_recovery_markers_are_cleanup_only() {
    let temporary = tempfile::tempdir().unwrap();
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Missing
    );

    write_marker(temporary.path(), r#"{"schema":2,"entries":[]}"#);
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::KnownEmpty
    );

    std::fs::create_dir(temporary.path().join("allocated-but-empty")).unwrap();
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::KnownEmpty
    );

    write_marker(
        temporary.path(),
        &format!("{LEGACY_MANIFEST_HEADER}\nactive_transfer=0\n"),
    );
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::KnownEmpty
    );
}

#[test]
fn remote_drive_task_invalid_or_declared_recovery_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    write_marker(temporary.path(), r#"{"schema":2,"entries":["#);
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Recovery
    );

    write_marker(
        temporary.path(),
        r#"{
  "schema": 2,
  "entries": [{
    "name": "note.md",
    "local": "missing-during-atomic-save.md",
    "remote": "/notes/note.md",
    "dirty": true,
    "uploading": false
  }]
}"#,
    );
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Recovery
    );
}

#[test]
fn remote_drive_task_empty_marker_with_real_payload_is_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    write_marker(temporary.path(), r#"{"schema":2,"entries":[]}"#);
    std::fs::write(temporary.path().join("downloaded-note.md"), b"unsaved").unwrap();
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Recovery
    );
}
