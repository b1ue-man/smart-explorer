use super::super::move_guard::source_snapshot_entry;
use super::super::staging::{open_regular_source, unchanged};
use super::*;

fn base(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("se_copy_{name}_{}", std::process::id()))
}

fn transfer(
    source: &Path,
    target: &Path,
    root: &Path,
    conflict: Conflict,
    mode: CopyMode,
    cancel: &AtomicBool,
) -> io::Result<TransferResult> {
    transfer_file(source, target, root, conflict, mode, cancel)
}

#[test]
fn overwrite_self_is_rejected_without_truncation() {
    let base = base("self");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let file = base.join("file.txt");
    std::fs::write(&file, b"keep").unwrap();
    let result = transfer(
        &file,
        &file,
        &base,
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(false),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn overwrite_hard_link_alias_is_rejected_without_data_loss() {
    let base = base("alias");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let source = base.join("source.txt");
    let alias = base.join("alias.txt");
    std::fs::write(&source, b"keep").unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let result = transfer(
        &source,
        &alias,
        &base,
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(false),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&source).unwrap(), b"keep");
    assert_eq!(std::fs::read(&alias).unwrap(), b"keep");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn canceled_overwrite_keeps_existing_destination() {
    let base = base("cancel");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let source = base.join("source.txt");
    let target = base.join("target.txt");
    std::fs::write(&source, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let result = transfer(
        &source,
        &target,
        &base,
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(true),
    )
    .unwrap();
    assert!(matches!(result, TransferResult::Canceled));
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn no_replace_commit_preserves_intervening_destination() {
    let base = base("race");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let staged = base.join("staged");
    let target = base.join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"intervening").unwrap();
    let result = platform::commit_staged(&staged, &target, false, true);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&target).unwrap(), b"intervening");
    assert_eq!(std::fs::read(&staged).unwrap(), b"new");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn changed_move_source_is_not_deleted() {
    let base = base("source_swap");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let source = base.join("source");
    std::fs::write(&source, b"original").unwrap();
    let secured = quarantine_source(&source).unwrap();
    std::fs::write(&source, b"replacement").unwrap();

    let error = restore_quarantine(&secured).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
    assert_eq!(std::fs::read(&secured.path).unwrap(), b"original");
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn destination_symlink_ancestor_is_rejected() {
    use std::os::unix::fs::symlink;

    let base = base("escape");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("root")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    symlink(base.join("outside"), base.join("root/link")).unwrap();
    let source = base.join("source");
    std::fs::write(&source, b"data").unwrap();
    let target = base.join("root/link/victim");
    let result = transfer(
        &source,
        &target,
        &base.join("root"),
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(false),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert!(!base.join("outside/victim").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn review_task_copy_stage_is_private_before_receiving_content() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir().unwrap();
    let (path, writer) =
        super::super::staging::create_temp_sibling(&fixture.path().join("target")).unwrap();
    assert_eq!(
        writer.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(writer.metadata().unwrap().len(), 0);
    drop(writer);
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn review_task_staged_copy_overwrite_keeps_private_destination_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    let target = fixture.path().join("target");
    std::fs::write(&source, b"new").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o6754)).unwrap();
    std::fs::write(&target, b"old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    let result = transfer(
        &source,
        &target,
        fixture.path(),
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(matches!(result, TransferResult::Completed));
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn review_task_copy_identity_never_follows_a_replaced_stage_link() {
    use std::os::unix::fs::symlink;

    let fixture = tempfile::tempdir().unwrap();
    let staged = fixture.path().join("stage");
    std::fs::write(&staged, b"stage").unwrap();
    let file = std::fs::File::open(&staged).unwrap();
    let identity = platform::file_identity(&file).unwrap();
    let moved = fixture.path().join("moved");
    std::fs::rename(&staged, &moved).unwrap();
    symlink(&moved, &staged).unwrap();
    assert!(!platform::path_matches_identity(&staged, identity).unwrap());
}

#[cfg(unix)]
#[test]
fn review_task_copy_source_snapshot_refuses_a_fifo() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let fixture = tempfile::tempdir().unwrap();
    let fifo = fixture.path().join("pipe");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: NUL-terminated fixture path and ordinary mode bits.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let error = source_snapshot_path(&fifo).unwrap_err();
    assert_eq!(
        crate::local_access::NotRegular::of(&error),
        Some(crate::local_access::NotRegular::Special),
    );
}

fn fixture(name: &str) -> PathBuf {
    let dir = base(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The path comparison after a copy reads what the handle read at the start.
#[test]
fn speed_task_copy_entry_snapshot_equals_the_open_snapshot() {
    let dir = fixture("entry_same");
    let source = dir.join("source");
    std::fs::write(&source, b"abcd").unwrap();
    let (_reader, before) = open_regular_source(&source).unwrap();
    assert_eq!(source_snapshot_entry(&source).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file renamed over the source after the open is not the opened file:
/// the final comparison refuses it, so nothing of it is published.
#[cfg(unix)]
#[test]
fn speed_task_copy_swapped_source_is_refused_after_the_copy() {
    let dir = fixture("entry_swap");
    let source = dir.join("source");
    let other = dir.join("other");
    std::fs::write(&source, b"abcd").unwrap();
    std::fs::write(&other, b"wxyz").unwrap();
    let (reader, before) = open_regular_source(&source).unwrap();
    std::fs::rename(&other, &source).unwrap();
    let verdict = unchanged(&source, &reader, &before);
    assert!(
        matches!(verdict, Err(LocalFailure::Source(_))),
        "{verdict:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Growth of the source while it is copied is still noticed (length and
/// modification time are part of the comparison).
#[test]
fn speed_task_copy_grown_source_is_refused_after_the_copy() {
    use std::io::Write;
    let dir = fixture("entry_grow");
    let source = dir.join("source");
    std::fs::write(&source, b"abcd").unwrap();
    let (reader, before) = open_regular_source(&source).unwrap();
    let mut append = std::fs::OpenOptions::new()
        .append(true)
        .open(&source)
        .unwrap();
    append.write_all(b"more").unwrap();
    drop(append);
    let verdict = unchanged(&source, &reader, &before);
    assert!(
        matches!(verdict, Err(LocalFailure::Source(_))),
        "{verdict:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The one-`lstat` comparison never accepts a link, a folder or a pipe at
/// the source path.
#[cfg(unix)]
#[test]
fn speed_task_copy_entry_snapshot_refuses_links_folders_and_pipes() {
    use crate::local_access::NotRegular;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let dir = fixture("entry_kinds");
    let file = dir.join("file");
    std::fs::write(&file, b"abcd").unwrap();
    let link = dir.join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let folder = dir.join("folder");
    std::fs::create_dir(&folder).unwrap();
    let fifo = dir.join("pipe");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: NUL-terminated fixture path and ordinary mode bits.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    for (path, expected) in [
        (link, NotRegular::Link),
        (folder, NotRegular::Directory),
        (fifo, NotRegular::Special),
    ] {
        let error = source_snapshot_entry(&path).unwrap_err();
        assert_eq!(NotRegular::of(&error), Some(expected), "{}", path.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn copy_new(source: &Path, target: &Path, calls: &mut Vec<u64>) -> LocalOutcome {
    let cancel = AtomicBool::new(false);
    let request = LocalRequest {
        source,
        target,
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
        cancel: &cancel,
        parallel: 1,
        probe: None,
    };
    transfer_local(&request, &mut |bytes| calls.push(bytes)).unwrap()
}

/// A small new file is one kernel chunk: one progress call, its mode
/// copied, no stage left behind, the source untouched.
#[cfg(unix)]
#[test]
fn speed_task_copy_small_file_is_one_chunk_with_its_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = fixture("small_chunk");
    let source = dir.join("source");
    let target = dir.join("target");
    std::fs::write(&source, vec![7u8; 4096]).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut calls = Vec::new();
    let outcome = copy_new(&source, &target, &mut calls);
    assert!(matches!(
        outcome,
        LocalOutcome::Completed { bytes: 4096, .. }
    ));
    assert_eq!(calls, vec![4096]);
    assert_eq!(std::fs::read(&target).unwrap(), vec![7u8; 4096]);
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["source", "target"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn speed_task_copy_empty_file_publishes_zero_bytes() {
    let dir = fixture("empty");
    let source = dir.join("source");
    let target = dir.join("target");
    std::fs::write(&source, b"").unwrap();
    let mut calls = Vec::new();
    let outcome = copy_new(&source, &target, &mut calls);
    assert!(matches!(outcome, LocalOutcome::Completed { bytes: 0, .. }));
    assert!(calls.is_empty());
    assert_eq!(std::fs::metadata(&target).unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file just over the kernel chunk (8 MiB) still takes a second call for
/// its tail; the short-chunk shortcut ends only at the real end.
#[cfg(unix)]
#[test]
fn speed_task_copy_file_over_one_chunk_copies_every_byte() {
    let dir = fixture("two_chunks");
    let source = dir.join("source");
    let target = dir.join("target");
    let size = 8 * 1024 * 1024 + 5;
    let content: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    std::fs::write(&source, &content).unwrap();
    let mut calls = Vec::new();
    let outcome = copy_new(&source, &target, &mut calls);
    assert!(matches!(outcome, LocalOutcome::Completed { .. }));
    assert_eq!(calls.iter().sum::<u64>(), size as u64);
    assert_eq!(calls.last().copied(), Some(5));
    assert_eq!(std::fs::read(&target).unwrap(), content);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A chunk of exactly 8 MiB is not the end: the zero-length probe follows.
#[cfg(unix)]
#[test]
fn speed_task_copy_file_of_exactly_one_chunk_is_complete() {
    let dir = fixture("one_chunk");
    let source = dir.join("source");
    let target = dir.join("target");
    let size = 8 * 1024 * 1024;
    std::fs::write(&source, vec![3u8; size]).unwrap();
    let mut calls = Vec::new();
    let outcome = copy_new(&source, &target, &mut calls);
    assert!(matches!(outcome, LocalOutcome::Completed { .. }));
    assert_eq!(calls.iter().sum::<u64>(), size as u64);
    assert_eq!(std::fs::metadata(&target).unwrap().len(), size as u64);
    let _ = std::fs::remove_dir_all(&dir);
}
