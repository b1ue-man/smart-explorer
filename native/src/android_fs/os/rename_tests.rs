use std::io;

use super::rename::{
    after_link_failure, checked_rename, fallback_for, flag_unsupported, remove_linked_source, Step,
};
use super::rename_no_replace;

#[test]
fn android_task_rename_fallback_decisions_follow_the_error_numbers() {
    for errno in [libc::EINVAL, libc::ENOSYS, libc::EOPNOTSUPP] {
        assert!(flag_unsupported(Some(errno)), "errno {errno}");
    }
    for errno in [libc::EEXIST, libc::EXDEV, libc::ENOENT, libc::EACCES] {
        assert!(!flag_unsupported(Some(errno)), "errno {errno}");
    }
    assert!(!flag_unsupported(None));

    assert_eq!(fallback_for(false), Step::HardLink);
    assert_eq!(fallback_for(true), Step::CheckedRename);

    assert_eq!(after_link_failure(Some(libc::EEXIST)), Step::Fail);
    for errno in [libc::EPERM, libc::ENOSYS, libc::EOPNOTSUPP, libc::EACCES] {
        assert_eq!(after_link_failure(Some(errno)), Step::CheckedRename);
    }
}

#[test]
fn android_task_rename_no_replace_moves_and_never_replaces() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source.txt");
    let moved = fixture.path().join("moved.txt");
    let taken = fixture.path().join("taken.txt");
    std::fs::write(&source, b"payload").unwrap();
    std::fs::write(&taken, b"keep").unwrap();

    rename_no_replace(&source, &moved).unwrap();
    assert!(!source.exists());
    assert_eq!(std::fs::read(&moved).unwrap(), b"payload");

    let error = rename_no_replace(&moved, &taken).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&moved).unwrap(), b"payload");
    assert_eq!(std::fs::read(&taken).unwrap(), b"keep");
}

#[test]
fn android_task_hard_link_step_is_create_only_and_drops_the_source() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source.txt");
    let destination = fixture.path().join("destination.txt");
    let taken = fixture.path().join("taken.txt");
    std::fs::write(&source, b"payload").unwrap();
    std::fs::write(&taken, b"keep").unwrap();

    let error = std::fs::hard_link(&source, &taken).unwrap_err();
    assert_eq!(after_link_failure(error.raw_os_error()), Step::Fail);
    assert_eq!(std::fs::read(&taken).unwrap(), b"keep");

    std::fs::hard_link(&source, &destination).unwrap();
    remove_linked_source(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(std::fs::read(&destination).unwrap(), b"payload");
}

#[test]
fn android_task_checked_rename_refuses_existing_names_and_moves_directories() {
    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path().join("folder");
    let moved = fixture.path().join("moved");
    let taken = fixture.path().join("taken");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("inner.txt"), b"inner").unwrap();
    std::fs::create_dir(&taken).unwrap();

    let error = checked_rename(&directory, &taken).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(directory.join("inner.txt").exists());

    checked_rename(&directory, &moved).unwrap();
    assert!(!directory.exists());
    assert_eq!(std::fs::read(moved.join("inner.txt")).unwrap(), b"inner");
}
