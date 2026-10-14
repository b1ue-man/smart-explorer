use super::*;
use crate::share::{
    export_config::ExportAccess,
    fs::{ShareExportConfig, SharedRoot},
};

fn export(root: &std::path::Path, access: ExportAccess) -> FsAccess {
    FsAccess::dynamic(ShareExportConfig {
        roots: vec![
            SharedRoot::new("Docs", root.to_string_lossy().replace('\\', "/")).with_access(access),
        ],
        ..Default::default()
    })
}

fn run(access: &FsAccess, path: &str) -> (io::Result<()>, FsRemoveTreeMessage) {
    let counters = RemovalCounters::default();
    let result = remove(access, path, &AtomicBool::new(false), &counters);
    (result, counters.done())
}

#[test]
fn delete_speed_task_host_counts_and_removes_a_nested_folder() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fixture.path().join("old/sub")).unwrap();
    std::fs::write(fixture.path().join("old/a"), b"a").unwrap();
    std::fs::write(fixture.path().join("old/sub/b"), b"b").unwrap();
    std::fs::write(fixture.path().join("kept"), b"kept").unwrap();
    let access = export(fixture.path(), ExportAccess::ReadWrite);
    let (result, done) = run(&access, "/Docs/old");
    result.unwrap();
    // a, sub, b and the folder: planned by the preflight, then removed.
    assert_eq!(
        done,
        FsRemoveTreeMessage::Done {
            planned: 4,
            removed: 4
        }
    );
    assert!(!fixture.path().join("old").exists());
    assert_eq!(std::fs::read(fixture.path().join("kept")).unwrap(), b"kept");
}

#[cfg(unix)]
#[test]
fn delete_speed_task_host_removes_a_folder_link_as_entry() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fixture.path().join("old")).unwrap();
    std::fs::create_dir_all(fixture.path().join("other")).unwrap();
    std::fs::write(fixture.path().join("other/kept"), b"kept").unwrap();
    std::os::unix::fs::symlink(
        fixture.path().join("other"),
        fixture.path().join("old/link"),
    )
    .unwrap();
    let access = export(fixture.path(), ExportAccess::ReadWrite);
    let (result, done) = run(&access, "/Docs/old");
    result.unwrap();
    assert_eq!(
        done,
        FsRemoveTreeMessage::Done {
            planned: 2,
            removed: 2
        }
    );
    assert!(!fixture.path().join("old").exists());
    assert_eq!(
        std::fs::read(fixture.path().join("other/kept")).unwrap(),
        b"kept"
    );

    // A link at the requested path itself is refused, its target stays.
    std::os::unix::fs::symlink(fixture.path().join("other"), fixture.path().join("alias")).unwrap();
    let (result, done) = run(&access, "/Docs/alias");
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        done,
        FsRemoveTreeMessage::Done {
            planned: 0,
            removed: 0
        }
    );
    assert!(fixture.path().join("alias").exists());
    assert!(fixture.path().join("other/kept").exists());
}

#[test]
fn delete_speed_task_host_refuses_export_roots_and_files() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("file"), b"file").unwrap();
    let access = export(fixture.path(), ExportAccess::ReadWrite);
    for root in ["/", "/Docs", "/Docs/"] {
        let (result, _) = run(&access, root);
        assert_eq!(
            result.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied,
            "{root}"
        );
    }
    let (result, _) = run(&access, "/Docs/file");
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read(fixture.path().join("file")).unwrap(), b"file");

    for (path, root) in [
        ("/Verbindungen", true),
        ("/Verbindungen/Server", true),
        ("/Verbindungen/Server/folder", false),
        ("/Docs/folder", false),
    ] {
        assert_eq!(
            crate::share::fs::is_export_root(path).unwrap(),
            root,
            "{path}"
        );
    }
}

#[test]
fn delete_speed_task_host_refuses_read_only_export() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fixture.path().join("old")).unwrap();
    std::fs::write(fixture.path().join("old/a"), b"a").unwrap();
    let access = export(fixture.path(), ExportAccess::ReadOnly);
    let (result, done) = run(&access, "/Docs/old");
    assert_eq!(
        result.unwrap_err().kind(),
        io::ErrorKind::ReadOnlyFilesystem
    );
    assert_eq!(
        done,
        FsRemoveTreeMessage::Done {
            planned: 0,
            removed: 0
        }
    );
    assert_eq!(std::fs::read(fixture.path().join("old/a")).unwrap(), b"a");
}

#[test]
fn delete_speed_task_host_stops_when_canceled() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fixture.path().join("old")).unwrap();
    std::fs::write(fixture.path().join("old/a"), b"a").unwrap();
    let access = export(fixture.path(), ExportAccess::ReadWrite);
    let counters = RemovalCounters::default();
    let error = remove(&access, "/Docs/old", &AtomicBool::new(true), &counters).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(counters.removed.load(Ordering::Acquire), 0);
    assert_eq!(std::fs::read(fixture.path().join("old/a")).unwrap(), b"a");
}

#[test]
fn delete_speed_task_host_never_reports_planned_below_removed() {
    let counters = RemovalCounters::default();
    counters.applying.store(true, Ordering::Release);
    // A non-local export has no preflight: only the folder itself is planned.
    counters.planned.store(1, Ordering::Release);
    counters.removed.store(3, Ordering::Release);
    assert_eq!(
        counters.progress(),
        FsRemoveTreeMessage::Progress {
            applying: true,
            planned: 3,
            removed: 3
        }
    );
    counters.planned.store(7, Ordering::Release);
    assert_eq!(
        counters.done(),
        FsRemoveTreeMessage::Done {
            planned: 7,
            removed: 3
        }
    );
}
