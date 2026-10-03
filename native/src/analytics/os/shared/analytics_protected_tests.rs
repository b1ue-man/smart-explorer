use super::*;
use crate::analytics::ProtectedOmission;
use std::path::PathBuf;

fn volume() -> (tempfile::TempDir, PathBuf) {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = std::fs::canonicalize(fixture.path()).expect("canonical");
    for dir in [
        "Android/data/own",
        "Android/data/foreign",
        "Android/obb",
        "DCIM",
    ] {
        std::fs::create_dir_all(root.join(dir)).expect("dir");
    }
    std::fs::write(root.join("Android/data/own/cache.bin"), [1u8; 100]).expect("own");
    std::fs::write(root.join("Android/data/foreign/x.bin"), [2u8; 5]).expect("foreign");
    std::fs::write(root.join("DCIM/a.jpg"), [3u8; 50]).expect("photo");
    (fixture, root)
}

fn deny_foreign(path: &Path) -> io::Result<()> {
    if path.ends_with("foreign") {
        Err(io::ErrorKind::PermissionDenied.into())
    } else {
        Ok(())
    }
}

fn omission(area: &Path, entries: u64) -> ProtectedOmission {
    ProtectedOmission {
        area: crate::analytics::os::display_path(area),
        entries,
    }
}

#[test]
fn android_background_task_protected_denials_keep_the_result_complete() {
    let (_fixture, root) = volume();
    let areas = ProtectedAreas::for_walk_in(&root, std::slice::from_ref(&root));
    let progress = Progress::default();
    let outcome = scan_in(&root, &progress, Some(&deny_foreign), Some(areas), None);
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    assert!(outcome.issues.is_empty());
    assert_eq!(outcome.permission_denied, 0);
    assert_eq!(
        outcome.protected,
        [
            omission(&root.join("Android/data"), 1),
            omission(&root.join("Android/obb"), 0),
        ]
    );
    // The readable own folder is measured; the denied one is not.
    let tree = outcome.tree.expect("tree");
    assert_eq!(tree.size, 150);
    assert_eq!(progress.files.load(Ordering::Relaxed), 2);
}

#[test]
fn android_background_task_protected_root_is_complete_and_empty() {
    let (_fixture, root) = volume();
    let volumes = [root.clone()];
    let gone = root.join("Android/data/gone");
    let areas = ProtectedAreas::for_walk_in(&gone, &volumes);
    let outcome = scan_in(&gone, &Progress::default(), None, Some(areas), None);
    assert_eq!(outcome.status, ScanStatus::Complete);
    assert!(outcome.issues.is_empty());
    assert_eq!(outcome.protected, [omission(&root.join("Android/data"), 1)]);
    let tree = outcome.tree.expect("an empty result, not a failure");
    assert_eq!((tree.size, tree.children.len()), (0, 0));

    let data = root.join("Android/data");
    let areas = ProtectedAreas::for_walk_in(&data, &volumes);
    let outcome = scan_in(
        &data,
        &Progress::default(),
        Some(&deny_foreign),
        Some(areas),
        None,
    );
    assert_eq!(outcome.status, ScanStatus::Complete);
    assert_eq!(outcome.protected, [omission(&data, 1)]);
    assert_eq!(outcome.tree.expect("tree").size, 100);
}

#[test]
fn android_background_task_protected_entry_errors_of_android_are_omissions() {
    let (_fixture, root) = volume();
    let android = root.join("Android");
    let areas = ProtectedAreas::for_walk_in(&root, std::slice::from_ref(&root));
    let progress = Progress::default();
    let diagnostics = Diagnostics::with_protected(areas);
    let budget = AnalyticsBudget::default();
    let traversal = Traversal {
        progress: &progress,
        diagnostics: &diagnostics,
        budget: &budget,
        parallel: false,
        guard: None,
    };
    let entry_error = |name: &str| -> io::Result<LocalEntry> {
        let path = crate::analytics::os::display_path(&android.join(name));
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{path}: No such file or directory"),
        ))
    };
    let entries = vec![
        entry_error("data"),
        entry_error("database"),
        Ok(LocalEntry {
            name: "readable.bin".into(),
            kind: EntryKind::File,
            size: 9,
            ..Default::default()
        }),
    ];
    let tree = scan_entries(
        &traversal,
        &android,
        "Android".into(),
        Ok(entries.into_iter()),
        1,
        false,
    );
    let outcome = diagnostics.finish(tree, false);
    assert_eq!(outcome.protected, [omission(&root.join("Android/data"), 1)]);
    assert_eq!(outcome.issues.len(), 1, "an unrelated entry stays an issue");
    assert!(outcome.issues[0].detail.contains("database"));
    assert_eq!(outcome.status, ScanStatus::Partial);
    assert_eq!(outcome.tree.expect("tree").size, 9);
}

#[test]
fn android_background_task_walks_without_volumes_keep_reporting_denials() {
    let (_fixture, root) = volume();
    let outcome = scan_in(
        &root,
        &Progress::default(),
        Some(&deny_foreign),
        Some(ProtectedAreas::default()),
        None,
    );
    assert_eq!(outcome.status, ScanStatus::Partial);
    assert_eq!(outcome.permission_denied, 1);
    assert!(outcome.issues[0].path.ends_with("foreign"));
    assert!(outcome.protected.is_empty());
    #[cfg(not(target_os = "android"))]
    assert_eq!(crate::analytics::os::default_scan_threads(), 2);
}
