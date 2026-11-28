//! Removable volumes on Windows (U15, U17, 2026-11-27).
use super::{remove_file_like, to_os};
use std::path::{Path, PathBuf};

#[test]
fn removable_task_vfs_path_of_a_bare_verbatim_volume_is_its_root() {
    let guid = "Volume{0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0}";
    assert_eq!(to_os("//?/E:"), PathBuf::from(r"\\?\E:\"));
    assert_eq!(to_os("//?/E:/"), PathBuf::from(r"\\?\E:\"));
    assert_eq!(
        to_os(&format!("//?/{guid}")),
        PathBuf::from(format!(r"\\?\{guid}\"))
    );
    assert_eq!(
        to_os(&format!("//?/{guid}/")),
        PathBuf::from(format!(r"\\?\{guid}\"))
    );
    // Paths below a root and plain drives keep their spelling.
    assert_eq!(
        to_os(&format!("//?/{guid}/DCIM")),
        PathBuf::from(format!(r"\\?\{guid}\DCIM"))
    );
    assert_eq!(to_os("//?/E:/DCIM"), PathBuf::from(r"\\?\E:\DCIM"));
    assert_eq!(to_os("E:"), PathBuf::from(r"E:\"));
    assert_eq!(to_os("E:/DCIM"), PathBuf::from(r"E:\DCIM"));
}

fn read_only_file(dir: &Path) -> PathBuf {
    let path = dir.join("r.txt");
    std::fs::write(&path, b"read only").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

fn fresh(base: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = base.join(format!("se-removable-rm-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn removable_task_read_only_file_is_removed() {
    let dir = fresh(&std::env::temp_dir());
    let path = read_only_file(&dir);
    remove_file_like(&path).unwrap();
    assert!(std::fs::symlink_metadata(&path).is_err());
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
#[ignore = "needs a FAT32 (SE_TEST_FAT32_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory on Windows; Suite crates-task"]
fn removable_task_read_only_file_is_removed_from_fat_volumes() {
    for variable in ["SE_TEST_FAT32_DIR", "SE_TEST_EXFAT_DIR"] {
        let base = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is not set"));
        let dir = fresh(Path::new(&base));
        let path = read_only_file(&dir);
        remove_file_like(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(std::fs::symlink_metadata(&path).is_err());
        std::fs::remove_dir(&dir).unwrap();
    }
}
