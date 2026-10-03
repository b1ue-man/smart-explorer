//! The shared task suite exercises handle-confined local walks through these
//! signals. Ordinary local readers retain their separate Explorer contract.
use std::ffi::OsStr;
use std::io::{self, Read, Write};

use super::{DirectoryHandle, EntryKind};

#[test]
fn review_task_directory_handles_keep_independent_listing_positions() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("one"), b"one").unwrap();
    std::fs::write(fixture.path().join("two"), b"two").unwrap();
    std::fs::create_dir(fixture.path().join("child")).unwrap();
    std::fs::write(fixture.path().join("child/inside"), b"inside").unwrap();
    let root = DirectoryHandle::open_root(fixture.path()).unwrap();
    let mut first = root.read_directory().unwrap();
    assert!(first.next().unwrap().is_ok());
    let names = root
        .read_directory()
        .unwrap()
        .map(|entry| entry.unwrap().name)
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 3);
    assert_eq!(first.count(), 2, "a second stream never advances the first");
    let child = root.open_child(OsStr::new("child")).unwrap();
    let entries = child
        .read_directory()
        .unwrap()
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, OsStr::new("inside"));
    assert_eq!(entries[0].kind, EntryKind::File);
    let mut content = String::new();
    child
        .open_regular_child(OsStr::new("inside"))
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert_eq!(content, "inside");
}

#[test]
fn review_task_directory_handles_refuse_paths_instead_of_child_names() {
    let fixture = tempfile::tempdir().unwrap();
    let root = DirectoryHandle::open_root(fixture.path()).unwrap();
    for name in ["", ".", "..", "../escape", "a/b", "a/", "/absolute"] {
        let error = root
            .open_child(OsStr::new(name))
            .err()
            .expect("not one child");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        let error = root.open_regular_child(OsStr::new(name)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
    }
}

#[test]
fn review_task_directory_handles_create_private_children_without_replacement() {
    let fixture = tempfile::tempdir().unwrap();
    let root = DirectoryHandle::open_root(fixture.path()).unwrap();
    let private = root.create_private_child(OsStr::new("private")).unwrap();
    assert!(private.metadata().unwrap().is_dir());
    let mut record = private.create_file_new(OsStr::new("record")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(private.metadata().unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(record.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
    record.write_all(b"private record").unwrap();
    record.sync_all().unwrap();
    let error = private.create_file_new(OsStr::new("record")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    let error = root.create_private_child(OsStr::new("private")).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    drop(record);
    assert_eq!(std::fs::read(fixture.path().join("private/record")).unwrap(), b"private record");
}

#[test]
fn review_task_private_handle_hardening_refuses_hardlinked_records() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("old-record");
    std::fs::write(&source, b"owned fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o666)).unwrap();
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_FLAG_OPEN_REPARSE_POINT,
            READ_CONTROL, WRITE_DAC,
        };
        options.access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC)
            .share_mode(FILE_SHARE_READ).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    std::fs::hard_link(&source, fixture.path().join("alias")).unwrap();
    let file = options.open(&source).unwrap();
    assert!(super::secure_private_handle(&file, false).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o7777, 0o666);
    }
    drop(file);
    std::fs::remove_file(fixture.path().join("alias")).unwrap();
    let file = options.open(&source).unwrap();
    super::secure_private_handle(&file, false).unwrap();
    super::secure_private_handle(&file, false).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o7777, 0o600);
    }
    DirectoryHandle::open_root(fixture.path()).unwrap().secure_private().unwrap();
}

#[test]
fn review_task_recycle_quarantine_restore_never_replaces_a_new_child() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("file");
    std::fs::write(&original, b"expected").unwrap();
    let root = DirectoryHandle::open_root(fixture.path()).unwrap();
    let expected = root.open_regular_child(OsStr::new("file")).unwrap();
    let mut captured = root.quarantine_regular_child(OsStr::new("file"), &expected).unwrap();
    assert!(!original.exists());
    let mut content = String::new();
    captured.file().read_to_string(&mut content).unwrap();
    assert_eq!(content, "expected");
    std::fs::write(&original, b"new child").unwrap();
    assert_eq!(captured.restore().unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&original).unwrap(), b"new child");
    assert!(captured.retained_location().is_file());
    std::fs::remove_file(&original).unwrap();
    captured.restore().unwrap();
    assert_eq!(std::fs::read(&original).unwrap(), b"expected");
}

#[test]
fn review_task_recycle_quarantine_moves_only_to_a_free_anchored_name() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir(fixture.path().join("destination")).unwrap();
    std::fs::write(fixture.path().join("file"), b"expected").unwrap();
    std::fs::write(fixture.path().join("destination/taken"), b"keep").unwrap();
    let root = DirectoryHandle::open_root(fixture.path()).unwrap();
    let destination = root.open_child(OsStr::new("destination")).unwrap();
    let expected = root.open_regular_child(OsStr::new("file")).unwrap();
    let mut captured = root.quarantine_regular_child(OsStr::new("file"), &expected).unwrap();
    assert_eq!(captured.move_to(&destination, OsStr::new("taken")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    captured.move_to(&destination, OsStr::new("free")).unwrap();
    assert_eq!(std::fs::read(fixture.path().join("destination/taken")).unwrap(), b"keep");
    assert_eq!(std::fs::read(fixture.path().join("destination/free")).unwrap(), b"expected");
}

#[test]
fn review_task_recycle_quarantine_refuses_a_replaced_expected_child() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("file"), b"expected").unwrap();
    let root = DirectoryHandle::open_root(fixture.path()).unwrap();
    let expected = root.open_regular_child(OsStr::new("file")).unwrap();
    std::fs::rename(fixture.path().join("file"), fixture.path().join("moved")).unwrap();
    std::fs::write(fixture.path().join("file"), b"replacement").unwrap();
    let error = root.quarantine_regular_child(OsStr::new("file"), &expected)
        .err()
        .expect("different object");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(fixture.path().join("file")).unwrap(), b"replacement");
}

#[cfg(unix)]
mod unix {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::local_access::NotRegular;

    #[test]
    fn review_task_directory_handles_refuse_links_specials_and_keep_raw_names() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::create_dir(fixture.path().join("child")).unwrap();
        std::fs::write(fixture.path().join("file"), b"content").unwrap();
        symlink("child", fixture.path().join("dir-link")).unwrap();
        symlink("file", fixture.path().join("file-link")).unwrap();
        let raw = OsStr::from_bytes(b"raw-\xff");
        std::fs::write(fixture.path().join(raw), b"raw").unwrap();
        let fifo_path = fixture.path().join("pipe");
        let fifo = CString::new(fifo_path.as_os_str().as_bytes()).unwrap();
        // SAFETY: NUL-terminated fixture path and ordinary permission bits.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let root = DirectoryHandle::open_root(fixture.path()).unwrap();
        let entries = root
            .read_directory()
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert!(entries.iter().any(|entry| entry.name == raw));
        assert!(entries.iter().any(|entry| {
            entry.name == OsStr::new("pipe") && entry.kind == EntryKind::Other
        }));
        let error = root.open_child(OsStr::new("dir-link")).err().unwrap();
        assert_eq!(NotRegular::of(&error), Some(NotRegular::Link));
        let error = root.open_regular_child(OsStr::new("file-link")).unwrap_err();
        assert_eq!(NotRegular::of(&error), Some(NotRegular::Link));
        let error = root.open_regular_child(OsStr::new("pipe")).unwrap_err();
        assert_eq!(NotRegular::of(&error), Some(NotRegular::Special));
    }

    #[test]
    fn review_task_directory_handles_stay_on_root_when_its_path_is_replaced() {
        let fixture = tempfile::tempdir().unwrap();
        let chosen = fixture.path().join("chosen");
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&chosen).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(chosen.join("file"), b"selected").unwrap();
        std::fs::write(outside.join("file"), b"outside").unwrap();
        symlink(&chosen, fixture.path().join("alias")).unwrap();
        let root = DirectoryHandle::open_root(&fixture.path().join("alias")).unwrap();
        std::fs::rename(&chosen, fixture.path().join("moved")).unwrap();
        symlink(&outside, &chosen).unwrap();
        let mut content = String::new();
        root.open_regular_child(OsStr::new("file"))
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "selected");
        assert_eq!(root.read_directory().unwrap().count(), 1);
        let watch_root = root.watch_path().unwrap();
        assert_eq!(std::fs::read(watch_root.join("file")).unwrap(), b"selected");
        let private = root.create_private_child(OsStr::new("private")).unwrap();
        private.create_file_new(OsStr::new("record")).unwrap();
        assert!(fixture.path().join("moved/private/record").is_file());
        assert!(!outside.join("private").exists());
    }

    #[test]
    fn review_task_recycle_quarantine_keeps_its_parent_anchor_after_root_rename() {
        let fixture = tempfile::tempdir().unwrap();
        let chosen = fixture.path().join("chosen");
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&chosen).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(chosen.join("file"), b"expected").unwrap();
        std::fs::write(outside.join("file"), b"outside").unwrap();
        let root = DirectoryHandle::open_root(&chosen).unwrap();
        let expected = root.open_regular_child(OsStr::new("file")).unwrap();
        let mut captured = root.quarantine_regular_child(OsStr::new("file"), &expected).unwrap();
        std::fs::rename(&chosen, fixture.path().join("moved")).unwrap();
        symlink(&outside, &chosen).unwrap();
        captured.restore().unwrap();
        assert_eq!(std::fs::read(fixture.path().join("moved/file")).unwrap(), b"expected");
        assert_eq!(std::fs::read(outside.join("file")).unwrap(), b"outside");
    }
}

#[cfg(windows)]
#[test]
fn review_task_directory_handles_pin_the_entered_windows_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let chosen = fixture.path().join("chosen");
    std::fs::create_dir(&chosen).unwrap();
    let root = DirectoryHandle::open_root(fixture.path()).unwrap();
    let child = root.open_child(OsStr::new("chosen")).unwrap();
    let renamed = fixture.path().join("renamed");
    assert!(std::fs::rename(&chosen, &renamed).is_err());
    assert!(chosen.is_dir() && !renamed.exists());
    drop(child);
    std::fs::rename(&chosen, &renamed).unwrap();
}
