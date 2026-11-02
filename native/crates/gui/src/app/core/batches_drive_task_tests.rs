//! B1: exercise the production snapshot and pinned-media decision boundary.
use super::*;

fn entry(root: &str, media: Option<&str>, attachment: &str) -> DriveEntry {
    DriveEntry {
        root: root.into(),
        identity: media.map(str::to_owned),
        incarnation: Some(attachment.into()),
        free: 10,
        total: 100,
    }
}

#[test]
fn batches_task_drive_replacement_and_reattachment_invalidate_only_affected_root() {
    let original = entry("/media/a", Some("volume-a"), "attachment-1");
    let independent = entry("/media/b", Some("volume-b"), "attachment-2");
    for replacement in [
        entry("/media/a", Some("volume-c"), "attachment-3"),
        entry("/media/a", Some("volume-a"), "attachment-4"),
    ] {
        let delta = diff(
            &[original.clone(), independent.clone()],
            &[replacement, independent.clone()],
        );
        assert_eq!(delta.removed, ["/media/a"]);
        assert_eq!(delta.added, ["/media/a"]);
    }
    let mut capacity_only = original.clone();
    capacity_only.free = 4;
    let delta = diff(&[original], &[capacity_only]);
    assert!(delta.removed.is_empty());
    assert!(delta.added.is_empty());
}

#[test]
fn batches_task_drive_disconnected_binding_requires_original_proven_media() {
    let original = entry("/media/a", Some("volume-a"), "attachment-1");
    let mut binding = binding_for(&[original], "/media/a/child").unwrap();
    binding.disconnected = true;
    assert!(!binding_available(&binding, &[]));
    assert!(!binding_available(
        &binding,
        &[entry("/media/a", Some("volume-b"), "replacement")]
    ));
    assert!(!binding_available(
        &binding,
        &[entry("/media/a", None, "unknown")]
    ));
    assert!(!binding_available(
        &binding,
        &[entry("/media/ab", Some("volume-a"), "wrong-root")]
    ));
    assert!(binding_available(
        &binding,
        &[entry("/media/a", Some("volume-a"), "reattached")]
    ));
    let mut unknown = binding_for(&[entry("/media/a", None, "empty")], "/media/a").unwrap();
    unknown.disconnected = true;
    assert!(!binding_available(
        &unknown,
        &[entry("/media/a", None, "empty-again")]
    ));
}

#[test]
fn batches_task_drive_binding_uses_deepest_root_and_literal_path_boundaries() {
    let entries = [
        entry("/", Some("system"), "one"),
        entry("/media/a", Some("usb"), "two"),
    ];
    assert_eq!(
        binding_for(&entries, "/media/a/child")
            .unwrap()
            .identity
            .as_deref(),
        Some("usb")
    );
    assert_eq!(
        binding_for(&entries, "/media/ab/child")
            .unwrap()
            .identity
            .as_deref(),
        Some("system")
    );
    assert!(contains("E:\\", "e:/child"));
    assert!(contains("\\\\server\\share", "\\\\SERVER\\SHARE\\child"));
    assert!(!contains("\\\\server\\share", "\\\\server\\share2\\child"));
    assert!(!contains("/media/A", "/media/a/child"));
}
