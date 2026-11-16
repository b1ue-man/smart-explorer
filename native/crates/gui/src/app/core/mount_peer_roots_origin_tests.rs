//! The mount dialog lists the roots of a device for a person who waits for
//! them: the handle it browses with has the origin `User` (plan 2026-11-14,
//! E4, metadata presence). The open needs a worker, so this reads the source
//! text of the entry.

#[test]
fn verbindungen_task_metadata_presence_the_mount_dialog_lists_roots_as_user() {
    // A Windows checkout holds the source with CRLF line ends
    // (`update_unsigned_ui_tests.rs` reads its source the same way).
    let source = include_str!("mount_peer_roots.rs").replace("\r\n", "\n");
    let from = source
        .find("pub(super) fn discover_peer_mount(")
        .expect("discovery entry");
    let rest = &source[from..];
    let entry = &rest[..rest.find("\n}\n").expect("end of the entry")];
    assert!(entry.contains("OperationOrigin::User"), "{entry}");
    assert!(entry.contains("open_share_backend_for(open_target.clone(), origin)"));
    assert!(!entry.contains("open_share_backend("));
}
