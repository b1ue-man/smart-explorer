//! Acceptance signals of M16 (2026-10-28): planned local trees on the
//! transfer engine keep the terminal's promises.
use super::transfer_tree;
use crate::cli::target::Target;
use crate::cli::tree_observer::TreeObserver;
use crate::cli::tree_ops::TreeRequest;
use crate::types::CopyMode;
use crate::vfs::{BackendHandle, LocalBackend};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "local-speed-task-tree-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create scratch folder");
        Self(path)
    }

    fn path(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().replace('\\', "/")
    }

    fn os(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    fn file(&self, rel: &str, bytes: &[u8]) {
        if let Some(parent) = self.os(rel).parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(self.os(rel), bytes).expect("write file");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn target(path: &str) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    Target::with_backend_key(backend, path.to_string(), "local")
}

fn run(
    src: &str,
    dst: &str,
    force: bool,
    mode: CopyMode,
) -> Result<Option<crate::cli::tree_ops::TreeOutcome>, String> {
    let (source, destination) = (target(src), target(dst));
    let meta = source.backend.stat(&source.path).expect("stat source");
    let observer = TreeObserver::default();
    let request = TreeRequest {
        recursive: true,
        force,
        resume: false,
        expected_root: Some(&meta),
        destination_check: None,
        observer: &observer,
    };
    transfer_tree(&source, &destination, dst, &request, mode)
}

/// `se cp -r src dst` into an existing `dst` folder: new files arrive,
/// nested folders (even empty ones) are created, existing files with other
/// names stay untouched.
#[test]
fn local_speed_task_tree_copy_merges_into_existing_folder_on_engine() {
    let scratch = Scratch::new("merge");
    scratch.file("src/a.txt", b"a");
    scratch.file("src/sub/b.txt", b"bb");
    fs::create_dir_all(scratch.os("src/empty")).expect("empty folder");
    scratch.file("dst/keep.txt", b"keep");
    let outcome = run(
        &scratch.path("src"),
        &scratch.path("dst"),
        false,
        CopyMode::Copy,
    )
    .expect("copy succeeds")
    .expect("engine path takes the command");
    assert_eq!(outcome.destination, scratch.path("dst"));
    assert_eq!(fs::read(scratch.os("dst/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(scratch.os("dst/sub/b.txt")).unwrap(), b"bb");
    assert!(scratch.os("dst/empty").is_dir());
    assert_eq!(fs::read(scratch.os("dst/keep.txt")).unwrap(), b"keep");
    assert_eq!(fs::read(scratch.os("src/a.txt")).unwrap(), b"a");
}

/// Without `--force` an existing destination file is refused by the plan
/// before anything is written: no folder, no file of the tree appears.
#[test]
fn local_speed_task_tree_copy_refuses_conflict_before_writing() {
    let scratch = Scratch::new("conflict");
    scratch.file("src/new/c.txt", b"c");
    scratch.file("src/a.txt", b"new");
    scratch.file("dst/a.txt", b"old");
    let error = run(
        &scratch.path("src"),
        &scratch.path("dst"),
        false,
        CopyMode::Copy,
    )
    .expect_err("conflict refused");
    assert!(error.contains("--force"), "{error}");
    assert_eq!(fs::read(scratch.os("dst/a.txt")).unwrap(), b"old");
    assert!(!scratch.os("dst/new").exists());
}

/// With `--force` the existing file is replaced by the source's bytes.
#[test]
fn local_speed_task_tree_copy_force_replaces_on_engine() {
    let scratch = Scratch::new("force");
    scratch.file("src/a.txt", b"new bytes");
    scratch.file("dst/a.txt", b"old");
    run(
        &scratch.path("src"),
        &scratch.path("dst"),
        true,
        CopyMode::Copy,
    )
    .expect("forced copy succeeds")
    .expect("engine path takes the command");
    assert_eq!(fs::read(scratch.os("dst/a.txt")).unwrap(), b"new bytes");
}

/// A move into an existing folder: every file arrives, the source tree is
/// gone afterwards and nothing is left under a quarantine name.
#[test]
fn local_speed_task_tree_move_removes_source_after_publication() {
    let scratch = Scratch::new("move");
    scratch.file("src/a.txt", b"a");
    scratch.file("src/sub/b.txt", b"bb");
    fs::create_dir_all(scratch.os("dst")).expect("destination folder");
    let outcome = run(
        &scratch.path("src"),
        &scratch.path("dst"),
        false,
        CopyMode::Move,
    )
    .expect("move succeeds")
    .expect("engine path takes the command");
    assert!(outcome.kept.is_empty());
    assert_eq!(fs::read(scratch.os("dst/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(scratch.os("dst/sub/b.txt")).unwrap(), b"bb");
    assert!(!scratch.os("src").exists());
    let leftovers: Vec<String> = fs::read_dir(scratch.os(""))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("smart-explorer"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// A move keeps the source folder that still holds a link it does not move.
#[cfg(unix)]
#[test]
fn local_speed_task_tree_move_keeps_folder_with_omitted_link() {
    let scratch = Scratch::new("move-link");
    scratch.file("src/a.txt", b"a");
    std::os::unix::fs::symlink("a.txt", scratch.os("src/link")).expect("link");
    fs::create_dir_all(scratch.os("dst")).expect("destination folder");
    let outcome = run(
        &scratch.path("src"),
        &scratch.path("dst"),
        false,
        CopyMode::Move,
    )
    .expect("move succeeds")
    .expect("engine path takes the command");
    assert_eq!(outcome.kept, vec![scratch.path("src")]);
    assert_eq!(outcome.omitted.len(), 1);
    assert!(fs::symlink_metadata(scratch.os("src/link")).is_ok());
    assert!(!scratch.os("src/a.txt").exists());
    assert_eq!(fs::read(scratch.os("dst/a.txt")).unwrap(), b"a");
}
