//! `--conflict update` through the terminal's path: insert, replace, skip,
//! keep the newer destination, keep both, and the options that exclude it.
use super::run;
use crate::cli::target::Target;
use crate::cli::tree_observer::TreeObserver;
use crate::types::CopyMode;
use crate::vfs::{BackendHandle, LocalBackend};
use clap::Parser;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

fn local(path: &Path) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    let text = path.to_string_lossy().replace('\\', "/");
    Target::with_backend_key(backend, text, "local")
}

fn write(path: &Path, bytes: &[u8], seconds: u64) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

#[test]
fn abgleich_task_cli_update_decides_each_same_named_file() {
    let root = tempfile::tempdir().unwrap();
    let (src, dst) = (root.path().join("src"), root.path().join("dst"));
    write(&src.join("new.txt"), b"new", 5_000);
    write(&src.join("sub/deep.txt"), b"deep", 5_000);
    write(&src.join("up.txt"), b"newer content", 6_000);
    write(&src.join("same.txt"), b"same", 5_000);
    write(&src.join("keep.txt"), b"from source", 1_000);
    write(&src.join("clash.txt"), b"abc", 2_000);
    write(&dst.join("up.txt"), b"old", 1_000);
    write(&dst.join("same.txt"), b"same", 5_000);
    write(&dst.join("keep.txt"), b"destination is newer", 9_000);
    write(&dst.join("clash.txt"), b"abcde", 2_000);

    let outcome = run(
        &local(&src),
        &local(&dst),
        true,
        CopyMode::Copy,
        &TreeObserver::default(),
        None,
    )
    .expect("update copy");

    assert_eq!(fs::read(dst.join("new.txt")).unwrap(), b"new");
    assert_eq!(fs::read(dst.join("sub/deep.txt")).unwrap(), b"deep");
    assert_eq!(fs::read(dst.join("up.txt")).unwrap(), b"newer content");
    assert_eq!(fs::read(dst.join("same.txt")).unwrap(), b"same");
    assert_eq!(
        fs::read(dst.join("keep.txt")).unwrap(),
        b"destination is newer"
    );
    assert_eq!(fs::read(dst.join("clash.txt")).unwrap(), b"abcde");
    assert_eq!(fs::read(dst.join("clash (2).txt")).unwrap(), b"abc");
    let counts = outcome.update.expect("the rule's decisions");
    assert_eq!(
        (
            counts.replaced,
            counts.skipped_equal,
            counts.kept_newer,
            counts.kept_both
        ),
        (1, 1, 1, 1)
    );
    let notices = outcome.notices().join("\n");
    assert!(notices.contains("1 neuere ersetzt"), "{notices}");
    assert!(notices.contains("1 gleiche übersprungen"), "{notices}");
}

#[test]
fn abgleich_task_cli_update_of_one_file_and_a_second_run_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.txt");
    let target = root.path().join("out/a.txt");
    write(&source, b"first", 3_000);
    let run_once = || {
        run(
            &local(&source),
            &local(&target),
            false,
            CopyMode::Copy,
            &TreeObserver::default(),
            None,
        )
        .expect("update copy")
    };
    run_once();
    assert_eq!(fs::read(&target).unwrap(), b"first");
    // Far newer than any copy made now, whether the copy keeps the source's time or not.
    write(&source, b"second version", 4_000_000_000);
    let again = run_once();
    assert_eq!(fs::read(&target).unwrap(), b"second version");
    assert_eq!(again.update.unwrap().replaced, 1);
    // A repeat must never number a new file, whatever the copy did to the time.
    let third = run_once();
    assert_eq!(fs::read_dir(root.path().join("out")).unwrap().count(), 1);
    let counts = third.update.unwrap();
    assert_eq!(counts.kept_both, 0, "no duplicate on a repeat");
}

#[test]
fn abgleich_task_cli_update_move_keeps_equal_sources_and_refuses_remote_moves() {
    let root = tempfile::tempdir().unwrap();
    let (src, dst) = (root.path().join("src"), root.path().join("dst"));
    write(&src.join("moves.txt"), b"moved", 5_000);
    write(&src.join("stays.txt"), b"same", 5_000);
    write(&dst.join("stays.txt"), b"same", 5_000);
    let outcome = run(
        &local(&src),
        &local(&dst),
        true,
        CopyMode::Move,
        &TreeObserver::default(),
        None,
    )
    .expect("update move");
    assert_eq!(fs::read(dst.join("moves.txt")).unwrap(), b"moved");
    assert!(
        !src.join("moves.txt").exists(),
        "a moved file leaves the source"
    );
    assert!(src.join("stays.txt").exists(), "an equal file is not moved");
    assert_eq!(
        outcome.kept.len(),
        1,
        "the source folder still holds a file"
    );
    assert_eq!(outcome.update.unwrap().skipped_equal, 1);
}

#[test]
fn abgleich_task_cli_update_refuses_a_second_folder_policy() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"a", 1_000);
    let error = run(
        &local(&src),
        &local(&root.path().join("dst")),
        true,
        CopyMode::Copy,
        &TreeObserver::default(),
        Some(crate::transfer::FolderConflict::KeepBoth),
    )
    .unwrap_err();
    assert!(error.contains("--folder-conflict"), "{error}");
}

#[test]
fn abgleich_task_cli_conflict_option_parses_and_excludes_force() {
    use crate::cli::Cli;
    assert!(Cli::try_parse_from(["se", "cp", "a", "b", "--conflict", "update"]).is_ok());
    assert!(Cli::try_parse_from(["se", "mv", "a", "b", "--conflict", "update"]).is_ok());
    assert!(Cli::try_parse_from(["se", "get", "a", "b", "--conflict", "update"]).is_ok());
    assert!(Cli::try_parse_from(["se", "cp", "a", "b", "--conflict", "bogus"]).is_err());
    assert!(
        Cli::try_parse_from(["se", "cp", "a", "b", "--conflict", "update", "--force"]).is_err()
    );
}
