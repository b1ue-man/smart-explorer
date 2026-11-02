//! Links and special files in `se search`, `se cp`, `se mv` (2026-10-12 batch,
//! docs/plaene/2026-10-12-links-in-se/plan.md).
#[cfg(unix)]
use super::entry_kind::EntryKind;
use super::links_task_backends::{NoAtomicRename, RemoteLinks};
use super::ops::{copy_observed, move_observed};
#[cfg(unix)]
use super::search::{search_spec, search_with};
use super::target::Target;
use super::tree_observer::TreeObserver;
use crate::vfs::{BackendHandle, LocalBackend};
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn target(backend: &BackendHandle, path: &str, key: &str) -> Target {
    Target::with_backend_key(backend.clone(), path.to_string(), key)
}

fn local() -> BackendHandle {
    Arc::new(LocalBackend::new("/"))
}

struct Temp(tempfile::TempDir);

impl Temp {
    fn new() -> Self {
        Self(
            tempfile::Builder::new()
                .prefix("se-links-task-")
                .tempdir()
                .unwrap(),
        )
    }

    fn path(&self, rel: &str) -> String {
        self.0.path().join(rel).to_string_lossy().replace('\\', "/")
    }

    fn file(&self, rel: &str, bytes: &[u8]) -> String {
        let path = self.path(rel);
        if let Some(parent) = std::path::Path::new(&path).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, bytes).unwrap();
        path
    }
}

/// `src/a.txt`, `src/sub/b.txt`, and the entries a tree never follows:
/// `link_file -> a.txt`, `link_dir -> sub`, `dangling -> missing`,
/// `sub/loop -> ..`, `fifo`.
#[cfg(unix)]
fn tree_with_links(temp: &Temp) -> String {
    use std::os::unix::fs::symlink;
    temp.file("src/a.txt", b"alpha");
    temp.file("src/sub/b.txt", b"beta");
    let src = temp.path("src");
    symlink("a.txt", format!("{src}/link_file")).unwrap();
    symlink("sub", format!("{src}/link_dir")).unwrap();
    symlink("missing", format!("{src}/dangling")).unwrap();
    symlink("..", format!("{src}/sub/loop")).unwrap();
    let fifo = std::ffi::CString::new(format!("{src}/fifo")).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    src
}

#[cfg(unix)]
#[test]
fn links_task_walk_search_reports_links_and_specials_without_entering_them() {
    let temp = Temp::new();
    let src = tree_with_links(&temp);
    let backend = local();
    let mut found = Vec::new();
    search_with(
        &target(&backend, &src, "local"),
        &search_spec("", false, 0, true),
        &std::sync::atomic::AtomicBool::new(false),
        &mut |hit| found.push((hit.rel, hit.kind)),
    )
    .unwrap();
    found.sort_by(|left, right| left.0.cmp(&right.0));
    let expected = [
        ("a.txt", EntryKind::File),
        ("dangling", EntryKind::Link),
        ("fifo", EntryKind::Special),
        ("link_dir", EntryKind::Link),
        ("link_file", EntryKind::Link),
        ("sub", EntryKind::Dir),
        ("sub/b.txt", EntryKind::File),
        ("sub/loop", EntryKind::Link),
    ];
    let found: Vec<(&str, EntryKind)> = found
        .iter()
        .map(|(rel, kind)| (rel.as_str(), *kind))
        .collect();
    assert_eq!(found, expected);

    // Without `--dirs` directories are left out, links still match by name.
    let mut names = Vec::new();
    search_with(
        &target(&backend, &src, "local"),
        &search_spec("link", false, 0, false),
        &std::sync::atomic::AtomicBool::new(false),
        &mut |hit| names.push(hit.rel),
    )
    .unwrap();
    names.sort();
    assert_eq!(names, ["link_dir", "link_file"]);
}

#[cfg(unix)]
#[test]
fn links_task_recursive_copy_skips_links_and_specials_and_reports_them() {
    let temp = Temp::new();
    let src = tree_with_links(&temp);
    let dst = temp.path("dst");
    let backend = local();
    let outcome = copy_observed(
        &target(&backend, &src, "local"),
        &target(&backend, &dst, "local"),
        true,
        false,
        &TreeObserver::default(),
    )
    .unwrap();

    assert_eq!(outcome.destination, dst);
    assert_eq!(fs::read(format!("{dst}/a.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(format!("{dst}/sub/b.txt")).unwrap(), b"beta");
    for skipped in ["link_file", "link_dir", "dangling", "fifo", "sub/loop"] {
        assert!(
            fs::symlink_metadata(format!("{dst}/{skipped}")).is_err(),
            "{skipped} must not be recreated"
        );
    }
    let mut omitted: Vec<(String, EntryKind)> = outcome
        .omitted
        .iter()
        .map(|omission| (omission.path.replacen(&src, "", 1), omission.kind))
        .collect();
    omitted.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        omitted,
        [
            ("/dangling".to_string(), EntryKind::Link),
            ("/fifo".to_string(), EntryKind::Special),
            ("/link_dir".to_string(), EntryKind::Link),
            ("/link_file".to_string(), EntryKind::Link),
            ("/sub/loop".to_string(), EntryKind::Link),
        ]
    );
    assert!(outcome.kept.is_empty());
    assert_eq!(outcome.notices().len(), 5);
}

#[cfg(unix)]
#[test]
fn links_task_cross_connection_move_keeps_directories_holding_skipped_entries() {
    let temp = Temp::new();
    let src = tree_with_links(&temp);
    temp.file("src/plain/c.txt", b"gamma");
    let dst = temp.path("dst");
    let backend = local();
    // Different keys: no rename, copy then delete.
    let outcome = move_observed(
        &target(&backend, &src, "source-connection"),
        &target(&backend, &dst, "destination-connection"),
        true,
        false,
        &TreeObserver::default(),
    )
    .unwrap();

    assert_eq!(fs::read(format!("{dst}/sub/b.txt")).unwrap(), b"beta");
    assert_eq!(fs::read(format!("{dst}/plain/c.txt")).unwrap(), b"gamma");
    assert!(!std::path::Path::new(&format!("{src}/a.txt")).exists());
    assert!(!std::path::Path::new(&format!("{src}/sub/b.txt")).exists());
    assert!(!std::path::Path::new(&format!("{src}/plain")).exists());
    assert!(fs::symlink_metadata(format!("{src}/sub/loop"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(format!("{src}/fifo")).is_ok());
    assert_eq!(outcome.kept, [src.clone(), format!("{src}/sub")]);
    assert_eq!(outcome.omitted.len(), 5);
}

#[cfg(unix)]
#[test]
fn links_task_same_connection_directory_move_renames_with_links_inside() {
    let temp = Temp::new();
    let src = tree_with_links(&temp);
    let dst = temp.path("renamed");
    let backend = local();
    let outcome = move_observed(
        &target(&backend, &src, "local"),
        &target(&backend, &dst, "local"),
        true,
        false,
        &TreeObserver::default(),
    )
    .unwrap();

    assert_eq!(outcome.destination, dst);
    assert!(outcome.omitted.is_empty());
    assert!(!std::path::Path::new(&src).exists());
    assert_eq!(
        fs::read_link(format!("{dst}/sub/loop")).unwrap(),
        std::path::PathBuf::from("..")
    );
    assert_eq!(fs::read(format!("{dst}/link_file")).unwrap(), b"alpha");
}

#[cfg(unix)]
#[test]
fn links_task_link_moves_by_rename_and_is_refused_across_connections() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    temp.file("real.txt", b"real");
    let link = temp.path("link.txt");
    symlink("real.txt", &link).unwrap();
    let backend = local();

    let error = move_observed(
        &target(&backend, &link, "one"),
        &target(&backend, &temp.path("elsewhere.txt"), "other"),
        false,
        false,
        &TreeObserver::default(),
    )
    .unwrap_err();
    assert!(error.contains("a link moves only by renaming"), "{error}");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());

    let renamed = temp.path("renamed.txt");
    move_observed(
        &target(&backend, &link, "local"),
        &target(&backend, &renamed, "local"),
        false,
        false,
        &TreeObserver::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read_link(&renamed).unwrap(),
        std::path::PathBuf::from("real.txt")
    );
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read(temp.path("real.txt")).unwrap(), b"real");
}

#[cfg(unix)]
#[test]
fn links_task_named_local_links_are_followed_for_source_and_destination() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let src = tree_with_links(&temp);
    let backend = local();

    // A link to a file as source: its content, under the link's name.
    let into = temp.path("into");
    fs::create_dir(&into).unwrap();
    let outcome = copy_observed(
        &target(&backend, &format!("{src}/link_file"), "local"),
        &target(&backend, &into, "local"),
        false,
        false,
        &TreeObserver::default(),
    )
    .unwrap();
    assert_eq!(outcome.destination, format!("{into}/link_file"));
    assert_eq!(fs::read(format!("{into}/link_file")).unwrap(), b"alpha");

    // A link to a directory as source: the tree it leads to.
    copy_observed(
        &target(&backend, &format!("{src}/link_dir"), "local"),
        &target(&backend, &into, "local"),
        true,
        false,
        &TreeObserver::default(),
    )
    .unwrap();
    assert_eq!(fs::read(format!("{into}/link_dir/b.txt")).unwrap(), b"beta");
    assert!(fs::symlink_metadata(format!("{into}/link_dir/loop")).is_err());

    // A link to a directory as destination, and a link among its ancestors.
    let real = temp.path("real-dir");
    fs::create_dir(&real).unwrap();
    let via = temp.path("via");
    symlink(&real, &via).unwrap();
    copy_observed(
        &target(&backend, &format!("{src}/a.txt"), "local"),
        &target(&backend, &via, "local"),
        false,
        false,
        &TreeObserver::default(),
    )
    .unwrap();
    copy_observed(
        &target(&backend, &format!("{src}/sub"), "local"),
        &target(&backend, &format!("{via}/new/deeper"), "local"),
        true,
        false,
        &TreeObserver::default(),
    )
    .unwrap();
    assert_eq!(fs::read(format!("{real}/a.txt")).unwrap(), b"alpha");
    assert_eq!(
        fs::read(format!("{real}/new/deeper/b.txt")).unwrap(),
        b"beta"
    );
    assert!(!fs::symlink_metadata(format!("{real}/new"))
        .unwrap()
        .file_type()
        .is_symlink());

    // A link to a file as destination is not written through.
    let error = copy_observed(
        &target(&backend, &format!("{src}/sub/b.txt"), "local"),
        &target(&backend, &format!("{src}/link_file"), "local"),
        false,
        true,
        &TreeObserver::default(),
    )
    .unwrap_err();
    assert!(error.contains("link to a file"), "{error}");
    assert_eq!(fs::read(format!("{src}/a.txt")).unwrap(), b"alpha");
}

#[test]
fn links_task_rename_without_no_replace_support_falls_back_to_copy_and_delete() {
    let temp = Temp::new();
    let src = temp.file("dir/a.txt", b"payload");
    temp.file("dir/sub/b.txt", b"tree");
    let backend = Arc::new(NoAtomicRename::default());
    let handle: BackendHandle = backend.clone();

    move_observed(
        &target(&handle, &src, "ftp-like"),
        &target(&handle, &temp.path("moved.txt"), "ftp-like"),
        false,
        false,
        &TreeObserver::default(),
    )
    .unwrap();
    move_observed(
        &target(&handle, &temp.path("dir"), "ftp-like"),
        &target(&handle, &temp.path("moved-dir"), "ftp-like"),
        true,
        false,
        &TreeObserver::default(),
    )
    .unwrap();

    assert_eq!(backend.refused.load(Ordering::Relaxed), 2);
    assert_eq!(fs::read(temp.path("moved.txt")).unwrap(), b"payload");
    assert_eq!(fs::read(temp.path("moved-dir/sub/b.txt")).unwrap(), b"tree");
    assert!(!std::path::Path::new(&temp.path("dir")).exists());
}

#[test]
fn links_task_remote_file_link_is_copied_by_content_and_directory_link_refused() {
    let temp = Temp::new();
    let remote: BackendHandle = Arc::new(RemoteLinks);
    let backend = local();
    let destination = temp.path("copied.txt");

    let outcome = copy_observed(
        &target(&remote, "/srv/link", "remote"),
        &target(&backend, &destination, "local"),
        false,
        false,
        &TreeObserver::default(),
    )
    .unwrap();
    assert_eq!(outcome.destination, destination);
    assert_eq!(fs::read(&destination).unwrap(), b"through the link");

    let error = copy_observed(
        &target(&remote, "/srv/dirlink", "remote"),
        &target(&backend, &temp.path("dir-copy"), "local"),
        true,
        false,
        &TreeObserver::default(),
    )
    .unwrap_err();
    assert!(error.contains("link to a directory on a remote"), "{error}");
    assert!(!std::path::Path::new(&temp.path("dir-copy")).exists());
}
