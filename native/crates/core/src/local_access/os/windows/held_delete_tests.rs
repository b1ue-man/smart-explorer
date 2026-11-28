use super::*;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
    FILE_WRITE_ATTRIBUTES,
};

#[test]
fn removable_task_read_only_bit_is_lifted_without_clearing_the_rest() {
    assert_eq!(
        writable_attributes(FILE_ATTRIBUTE_READONLY),
        FILE_ATTRIBUTE_NORMAL
    );
    // Hidden | system | archive stay; only READONLY goes.
    assert_eq!(
        writable_attributes(0x1 | 0x2 | 0x4 | 0x20),
        0x2 | 0x4 | 0x20
    );
    assert_eq!(writable_attributes(0x20), 0x20);
    for code in [1, 50, 87, 124] {
        assert!(disposition_ex_unsupported(&io::Error::from_raw_os_error(
            code
        )));
    }
    assert!(!disposition_ex_unsupported(&io::Error::from_raw_os_error(
        5
    )));
}

fn read_only_file(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"read only").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

fn delete_read_only(path: &Path) {
    let guard = std::fs::OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    delete_held(&guard, true).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    drop(guard);
    assert!(
        std::fs::symlink_metadata(path).is_err(),
        "{} is gone",
        path.display()
    );
}

fn fresh(base: &Path, label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = base.join(format!(
        "se-held-delete-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn removable_task_read_only_file_is_deleted_through_its_handle() {
    let dir = fresh(&std::env::temp_dir(), "ntfs");
    delete_read_only(&read_only_file(&dir, "r.txt"));
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
#[ignore = "needs a FAT32 (SE_TEST_FAT32_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory on Windows; Suite crates-task"]
fn removable_task_read_only_file_is_deleted_on_fat_volumes() {
    for variable in ["SE_TEST_FAT32_DIR", "SE_TEST_EXFAT_DIR"] {
        let base = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is not set"));
        let dir = fresh(Path::new(&base), "fat");
        delete_read_only(&read_only_file(&dir, "r.txt"));
        std::fs::remove_dir(&dir).unwrap();
    }
}
