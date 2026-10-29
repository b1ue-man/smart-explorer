//! The Windows small-file copy and the held rename (A7, 2026-10-29): what
//! `CopyFile2` kept is kept, a held stage cannot be moved by anyone else, the
//! rename never replaces, and a source with more than plain data stays with
//! `CopyFile2`.
use super::super::super::SourceFacts;
use super::*;
use windows_sys::Win32::Storage::FileSystem::{
    SetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY,
};

/// A long-name folder (canonical, so no 8.3 component as in `%TEMP%` on
/// the runners): the handle-name check answers there.
fn folder() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp folder");
    let path = std::fs::canonicalize(dir.path()).expect("canonical temp folder");
    (dir, path)
}

fn set_attributes(path: &Path, attributes: u32) {
    let wide = super::super::wide(path);
    // SAFETY: a NUL-terminated wide path.
    assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), attributes) }, 0);
}

fn facts(size: u64) -> Option<SourceFacts> {
    Some(SourceFacts {
        size,
        optical: false,
    })
}

#[test]
fn local_speed_task_windows_small_copy_keeps_bytes_times_and_attributes() {
    let (_dir, root) = folder();
    let source = root.join("source.txt");
    std::fs::write(&source, b"small file bytes").expect("source");
    set_attributes(&source, FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE);
    let reader = open_copy_source(&source).expect("a plain file opens on the fast way");
    let stage = root.join("source.txt.smart-explorer-test.part");
    let cancel = AtomicBool::new(false);
    let (bytes, file) = copy_from_reader(&source, &reader, &stage, &cancel, &mut |_| {}, facts(16))
        .expect("a small plain local file is copied here")
        .expect("copied")
        .expect("not canceled");
    assert_eq!(bytes, 16);
    let before = information(&reader).expect("source facts");
    let after = information(&file).expect("stage facts");
    assert_eq!(before.attributes, after.attributes);
    assert_eq!(
        (before.last_write_time.low, before.last_write_time.high),
        (after.last_write_time.low, after.last_write_time.high)
    );
    assert!(handle_has_path(&file, &stage));
    // Pinned: nobody else renames the held stage away.
    assert!(std::fs::rename(&stage, root.join("elsewhere")).is_err());
    let target = root.join("target.txt");
    commit_staged_held(&stage, &target, Some(&file), false, false).expect("published");
    assert!(handle_has_path(&file, &target));
    drop(file);
    assert_eq!(std::fs::read(&target).expect("target"), b"small file bytes");
    assert!(!stage.exists());
}

#[test]
fn local_speed_task_windows_held_rename_never_replaces() {
    let (_dir, root) = folder();
    let source = root.join("source.bin");
    std::fs::write(&source, b"new").expect("source");
    set_attributes(&source, FILE_ATTRIBUTE_READONLY);
    let reader = open_copy_source(&source).expect("opens");
    let stage = root.join("source.bin.smart-explorer-test.part");
    let (_, file) = copy_from_reader(
        &source,
        &reader,
        &stage,
        &AtomicBool::new(false),
        &mut |_| {},
        facts(3),
    )
    .expect("copied here")
    .expect("copied")
    .expect("not canceled");
    let target = root.join("target.bin");
    std::fs::write(&target, b"intervening").expect("target");
    let error = commit_staged_held(&stage, &target, Some(&file), false, false)
        .expect_err("an existing name is never replaced");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&target).expect("target"), b"intervening");
    // A read-only stage is still published by its handle.
    let free = root.join("free.bin");
    commit_staged_held(&stage, &free, Some(&file), false, false).expect("published");
    drop(file);
    assert_eq!(std::fs::read(&free).expect("published"), b"new");
    set_attributes(&free, FILE_ATTRIBUTE_ARCHIVE);
    set_attributes(&source, FILE_ATTRIBUTE_ARCHIVE);
}

#[test]
fn local_speed_task_windows_named_streams_keep_copyfile2() {
    let (_dir, root) = folder();
    let source = root.join("marked.txt");
    std::fs::write(&source, b"data").expect("source");
    std::fs::write(
        root.join("marked.txt:Zone.Identifier"),
        b"[ZoneTransfer]\r\nZoneId=3\r\n",
    )
    .expect("named stream");
    let reader = open_copy_source(&source).expect("opens");
    let stage = root.join("marked.txt.smart-explorer-test.part");
    assert!(copy_from_reader(
        &source,
        &reader,
        &stage,
        &AtomicBool::new(false),
        &mut |_| {},
        facts(4),
    )
    .is_none());
    assert!(!stage.exists());
}

#[test]
fn local_speed_task_windows_canceled_small_copy_leaves_no_stage() {
    let (_dir, root) = folder();
    let source = root.join("source.txt");
    std::fs::write(&source, vec![7u8; 300 * 1024]).expect("source");
    let reader = open_copy_source(&source).expect("opens");
    let stage = root.join("source.txt.smart-explorer-test.part");
    let copied = copy_from_reader(
        &source,
        &reader,
        &stage,
        &AtomicBool::new(true),
        &mut |_| {},
        facts(300 * 1024),
    )
    .expect("copied here")
    .expect("no error");
    assert!(copied.is_none());
    assert!(!stage.exists());
}

#[test]
fn local_speed_task_windows_folders_and_other_names_are_not_fast() {
    let (_dir, root) = folder();
    assert!(open_copy_source(&root).is_none());
    let file = root.join("named.txt");
    std::fs::write(&file, b"x").expect("file");
    let reader = open_copy_source(&file).expect("opens");
    assert!(handle_has_path(&reader, &file));
    let upper = root.join("NAMED.TXT");
    assert!(!handle_has_path(&reader, &upper));
    let link = root.join("second.txt");
    std::fs::hard_link(&file, &link).expect("hard link");
    assert!(!handle_has_path(&reader, &file));
}
