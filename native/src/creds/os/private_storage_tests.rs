use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};

#[test]
fn review_task_private_objects_are_restrictive_before_data_is_written() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("private/child");
    ensure_directory(&directory).unwrap();
    assert_eq!(std::fs::metadata(&directory).unwrap().permissions().mode() & 0o7777, 0o700);
    let file = create_file(&directory.join("secret")).unwrap();
    assert_eq!(file.metadata().unwrap().permissions().mode() & 0o7777, 0o600);
    assert_eq!(file.metadata().unwrap().len(), 0);
}

#[test]
fn review_task_private_objects_refuse_links_and_tighten_owned_old_modes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("old");
    std::fs::write(&path, b"private").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let file = open_file(&path, false).unwrap();
    assert_eq!(file.metadata().unwrap().permissions().mode() & 0o7777, 0o600);
    drop(file);
    symlink(&path, root.path().join("link")).unwrap();
    assert!(open_file(&root.path().join("link"), false).is_err());
    std::fs::hard_link(&path, root.path().join("hardlink")).unwrap();
    assert!(open_file(&path, false).is_err());
    assert!(create_file(&root.path().join("link")).is_err());
    let folder = root.path().join("folder");
    ensure_directory(&folder).unwrap();
    symlink(&folder, root.path().join("parent-link")).unwrap();
    assert!(create_file(&root.path().join("parent-link/secret")).is_err());
    let fifo = root.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(open_file(&fifo, false).is_err());
}
