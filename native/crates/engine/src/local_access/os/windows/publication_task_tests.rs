//! Windows publication contracts for the single remote task suite.
use super::*;
use std::{
    cell::{Cell, RefCell},
    fs,
    os::windows::fs::OpenOptionsExt,
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};

thread_local! {
    static NO_PARENT_PROOF: Cell<bool> = const { Cell::new(false) };
    static FAIL_FLUSH: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn namespace_support() -> io::Result<()> {
    if NO_PARENT_PROOF.with(Cell::get) {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "injected unavailable parent proof",
        ))
    } else {
        Ok(())
    }
}

pub fn before_flush(file: &File) -> io::Result<()> {
    let Some(expected) = FAIL_FLUSH.with(|value| value.borrow().clone()) else {
        return Ok(());
    };
    if expected == super::super::identity(file)? {
        return Err(io::Error::other("injected directory flush failure"));
    }
    Ok(())
}

pub struct Fault;
impl Fault {
    pub fn missing_parent_proof() -> Self {
        NO_PARENT_PROOF.with(|flag| assert!(!flag.replace(true)));
        Self
    }

    pub fn flush(file: &File) -> Self {
        FAIL_FLUSH.with(|value| {
            assert!(value
                .borrow_mut()
                .replace(super::super::identity(file).unwrap())
                .is_none());
        });
        Self
    }
}
impl Drop for Fault {
    fn drop(&mut self) {
        NO_PARENT_PROOF.with(|flag| flag.set(false));
        FAIL_FLUSH.with(|value| *value.borrow_mut() = None);
    }
}

fn raw_directory(path: &Path, sharing: u32) -> File {
    fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | 0x0010_0000)
        .share_mode(sharing)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap()
}

#[test]
fn batches_task_transfer_compat_windows_publication_flushes_root_and_native_child() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("child")).unwrap();
    let parent = crate::local_access::ConfinedDirectory::open_root(root.path()).unwrap();
    let child = parent.child(OsStr::new("child")).unwrap();
    for directory in [&parent, &child] {
        let before = directory.identity().unwrap();
        directory.confirm_publication_namespace().unwrap();
        assert_eq!(directory.identity().unwrap(), before);
    }
}

#[test]
fn batches_task_transfer_compat_windows_publication_reopens_the_held_directory_after_rename() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("chosen");
    let moved = root.path().join("moved");
    fs::create_dir(&original).unwrap();
    let held = raw_directory(
        &original,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    );
    let identity = super::super::identity(&held).unwrap();
    // Only this explicit test handle allows its directory name to change.
    // Production ConfinedDirectory pins continue to deny delete sharing.
    fs::rename(&original, &moved).unwrap();
    fs::create_dir(&original).unwrap();
    let foreign = raw_directory(
        &original,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    );
    let reopened = reopen_directory(&held).unwrap();
    assert_eq!(super::super::identity(&reopened).unwrap(), identity);
    assert_ne!(super::super::identity(&foreign).unwrap(), identity);
    confirm_namespace(&held).unwrap();
}

#[test]
fn batches_task_transfer_compat_windows_publication_rejects_wrong_type_and_write_reservation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("regular.bin");
    fs::write(&path, b"unchanged").unwrap();
    let file = File::open(&path).unwrap();
    assert_eq!(
        reopen_directory(&file).unwrap_err().kind(),
        io::ErrorKind::InvalidData,
    );
    let held = raw_directory(root.path(), FILE_SHARE_READ);
    let error = reopen_directory(&held).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"unchanged");
}

#[test]
fn batches_task_transfer_compat_windows_publication_never_promotes_non_ntfs_or_remote_labels() {
    assert!(namespace_supported("NTFS", 0));
    assert!(namespace_supported("ntfs", 0));
    for filesystem in [
        "ReFS", "FAT", "FAT12", "FAT16", "FAT32", "exFAT", "CDFS", "UDF", "",
    ] {
        assert!(!namespace_supported(filesystem, 0), "{filesystem}");
    }
    assert!(!namespace_supported("NTFS", 0x10));
    assert!(!namespace_supported("NTFS", 0x2000));
}

#[test]
fn batches_task_transfer_compat_windows_publication_propagates_directory_flush_failures() {
    let root = tempfile::tempdir().unwrap();
    let parent = crate::local_access::ConfinedDirectory::open_root(root.path()).unwrap();
    let identity = parent.identity().unwrap();
    let read_only = raw_directory(root.path(), FILE_SHARE_READ | FILE_SHARE_WRITE);
    let error = flush_directory(read_only).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(5), "{error}");
    {
        let _fault = Fault::flush(parent.file());
        let error = parent.confirm_publication_namespace().unwrap_err();
        assert!(error
            .to_string()
            .contains("injected directory flush failure"));
        assert_eq!(parent.identity().unwrap(), identity);
    }
    parent.confirm_publication_namespace().unwrap();
}
