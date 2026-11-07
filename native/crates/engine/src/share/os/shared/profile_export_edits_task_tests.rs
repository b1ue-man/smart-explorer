//! E11 (plan 2026-11-07), the profile edit: an export may not be added or
//! changed inside an unlocked vault drive; stored ones stay and can be
//! removed, and the folder around the drive stays exportable.
use super::*;
use crate::share::fs_host_policy_task_tests::{text, vault_fixture, Published};

fn profiles(roots: Vec<SharedRoot>) -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots = roots;
    profiles
}

/// M16-e.
#[test]
fn vault_task_export_edits_refuse_new_or_changed_roots_in_an_unlocked_vault() {
    let (_directory, folder, drive) = vault_fixture();
    let _published = Published::new(&drive);
    let root = SharedRoot::new("Tresor", text(&drive.join("sub")));

    let message = refuse_vault_exports(&[], &profiles(vec![root.clone()])).unwrap_err();
    assert_eq!(
        message,
        format!(
            "Ein entsperrter Tresor wird nicht freigegeben: {}",
            root.path
        )
    );
    // A root stored before stays as it is, and may be removed.
    refuse_vault_exports(&[root.clone()], &profiles(vec![root.clone()])).unwrap();
    refuse_vault_exports(&[root.clone()], &profiles(Vec::new())).unwrap();
    // A change of the stored root is an edit of an export in the drive.
    let mut renamed = root.clone();
    renamed.label = "Anders".into();
    assert!(refuse_vault_exports(&[root.clone()], &profiles(vec![renamed])).is_err());
    let writable = root
        .clone()
        .with_access(crate::share::ExportAccess::ReadWrite);
    assert!(refuse_vault_exports(&[root.clone()], &profiles(vec![writable])).is_err());
    // The folder around the drive and an unrelated folder stay exportable.
    let parent = SharedRoot::new("Docs", text(&folder));
    refuse_vault_exports(&[], &profiles(vec![parent])).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let other = SharedRoot::new("Anderswo", text(elsewhere.path()));
    refuse_vault_exports(&[], &profiles(vec![other])).unwrap();
}
