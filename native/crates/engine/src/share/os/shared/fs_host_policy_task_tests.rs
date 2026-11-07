use std::{
    io,
    path::{Path, PathBuf},
};

use super::{
    export_config::ExportAccess,
    fs_host_policy::{ensure_handle_allowed, TargetPolicy},
    fs_local_paths::{secure_local_target, to_os_path},
    fs_path_adapter::create_directory_link,
};
use crate::local_access::DirectoryHandle;

#[test]
fn review_task_host_read_only_is_a_rights_error_for_normal_write_paths() {
    let policy = TargetPolicy::new(ExportAccess::ReadOnly, false, false);
    policy.read("/mount/ordinary.txt").unwrap();
    assert_eq!(
        policy.write("/mount/ordinary.txt").unwrap_err().kind(),
        io::ErrorKind::ReadOnlyFilesystem,
    );
    assert_eq!(
        policy.read("/mount/.se-versions/x").unwrap_err().kind(),
        io::ErrorKind::PermissionDenied,
    );
}

#[test]
fn review_task_host_system_opt_in_never_opens_app_private_or_versions() {
    let root = crate::support_dirs::app_data_dir()
        .to_string_lossy()
        .replace('\\', "/");
    let policy = TargetPolicy::new(ExportAccess::ReadWrite, true, true);
    assert!(policy.read(&format!("{root}/identity.json")).is_err());
    assert!(policy.write(&format!("{root}/identity.json")).is_err());
    assert!(policy.write("/export/.se-versions/old").is_err());
}

#[test]
fn local_target_stays_under_root() {
    let root = std::env::temp_dir().join(format!("se-share-root-{}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let root_s = root.to_string_lossy().replace('\\', "/");
    let p = secure_local_target(&root_s, &["sub".to_string(), "file.txt".to_string()]).unwrap();
    let p = to_os_path(&p);
    let parent = p.parent().unwrap().canonicalize().unwrap();
    assert!(parent.starts_with(root.canonicalize().unwrap()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn symlink_escape_is_blocked_when_supported() {
    let base = std::env::temp_dir().join(format!("se-share-symlink-{}", std::process::id()));
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let link = root.join("link");

    if create_directory_link(&outside, &link).is_ok() {
        let root_s = root.to_string_lossy().replace('\\', "/");
        assert!(secure_local_target(&root_s, &["link".to_string()]).is_err());
    }
    let _ = std::fs::remove_dir_all(base);
}

/// Z-H2: a folder proven outside the private roots is remembered by path and
/// identity; replacing it with a link out of the export, or a link into the
/// app's private data, is still refused on the very next request.
#[test]
fn speed_task_share_host_policy_cache_keeps_link_confinement() {
    let base = std::env::temp_dir().join(format!("se-share-zh2-{}", std::process::id()));
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("file.txt"), b"outside").unwrap();
    let root = root.canonicalize().unwrap();
    let link = root.join("a");
    let root_s = root.to_string_lossy().replace('\\', "/");
    let policy = TargetPolicy::new(ExportAccess::ReadWrite, false, true);
    let policy = policy.with_root(&root_s);
    let inside = format!("{root_s}/a/file.txt");
    let rest = ["a".to_string(), "file.txt".to_string()];
    // Proven once (and remembered).
    policy.read(&inside).unwrap();
    policy.read(&inside).unwrap();
    std::fs::remove_dir(&link).unwrap();
    if create_directory_link(&outside, &link).is_ok() {
        assert!(policy.read(&inside).is_err());
        assert!(secure_local_target(&root_s, &rest).is_err());
        remove_link(&link);
        let private = crate::support_dirs::app_data_dir();
        if create_directory_link(&private, &link).is_ok() {
            assert!(policy.read(&inside).is_err());
            assert!(policy.write(&inside).is_err());
            remove_link(&link);
        }
    }
    super::fs_host_policy::forget_verified_directories();
    let _ = std::fs::remove_dir_all(base);
}

fn remove_link(link: &std::path::Path) {
    if std::fs::remove_dir(link).is_err() {
        std::fs::remove_file(link).unwrap();
    }
}

#[test]
fn speed_task_share_host_paths_refuse_parent_segments() {
    let base = std::env::temp_dir().join(format!("se-share-zh2-dots-{}", std::process::id()));
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(base.join("sibling")).unwrap();
    let root_s = root.to_string_lossy().replace('\\', "/");
    let sibling = secure_local_target(&root_s, &["..".to_string(), "sibling".to_string()]);
    assert!(sibling.is_err());
    let missing = secure_local_target(&root_s, &["..".to_string(), "missing".to_string()]);
    assert!(missing.is_err());
    let _ = std::fs::remove_dir_all(base);
}

const VAULT_SITE: &str = "0123456789abcdef0123456789abcdef";

/// A vault drive published for as long as the value lives (the table is
/// process-wide: a failing test must not leave its drive behind). The vault
/// tests of the other Share modules use it too.
pub(in crate::share) struct Published(PathBuf);

impl Published {
    pub(in crate::share) fn new(drive: &Path) -> Self {
        crate::vault::publish_vault_mount(VAULT_SITE, drive);
        Self(drive.to_path_buf())
    }
}

impl Drop for Published {
    fn drop(&mut self) {
        crate::vault::withdraw_vault_mount(VAULT_SITE, &self.0);
    }
}

/// An export folder `A` (canonical) with the folder `A/T` that a vault
/// drive shows, `A/T/geheim.txt`, `A/T/sub` and the ordinary `A/frei.txt`.
pub(in crate::share) fn vault_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().canonicalize().unwrap();
    std::fs::create_dir_all(folder.join("T/sub")).unwrap();
    std::fs::write(folder.join("T/geheim.txt"), b"klartext").unwrap();
    std::fs::write(folder.join("frei.txt"), b"frei").unwrap();
    let drive = folder.join("T");
    (directory, folder, drive)
}

pub(in crate::share) fn text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// M16-a (E11, plan 2026-11-07): what an unlocked vault drive shows is no
/// file of the export for any operation; the folders around it and the
/// ordinary files stay as they were, and the drive is free again once it
/// ended.
#[test]
fn vault_task_host_policy_refuses_an_unlocked_vault_drive_and_frees_it_when_it_ends() {
    let (_directory, folder, drive) = vault_fixture();
    let secret = text(&drive.join("geheim.txt"));
    let free = text(&folder.join("frei.txt"));
    let published = Published::new(&drive);
    let policy = TargetPolicy::new(ExportAccess::ReadWrite, false, true).with_root(&text(&folder));
    for refused in [
        policy.read(&secret),
        policy.write(&secret),
        policy.read(&text(&drive)),
        policy.destructive(&text(&folder)),
    ] {
        assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
    assert!(!policy.visible(&text(&drive), false));
    // The ordinary file next to the drive and the folder itself stay.
    policy.read(&free).unwrap();
    policy.write(&free).unwrap();
    assert!(policy.visible(&free, false));
    assert!(policy.visible(&text(&folder), false));

    drop(published);
    let policy = TargetPolicy::new(ExportAccess::ReadWrite, false, true).with_root(&text(&folder));
    policy.read(&secret).unwrap();
    assert!(policy.visible(&text(&drive), false));
}

/// M16-b: a held directory object inside a vault drive is refused by its
/// identity; a drive this process cannot reach (ended, another user's
/// mount) turns no ordinary request into an error.
#[test]
fn vault_task_host_handle_check_refuses_inside_a_vault_drive_and_ignores_a_missing_one() {
    let (_directory, folder, drive) = vault_fixture();
    let inside = DirectoryHandle::open_root(&drive.join("sub")).unwrap();
    let outside = DirectoryHandle::open_root(&folder).unwrap();
    let published = Published::new(&drive);
    assert_eq!(
        ensure_handle_allowed(&inside).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    ensure_handle_allowed(&outside).unwrap();

    drop(published);
    ensure_handle_allowed(&inside).unwrap();
    let _missing = Published::new(&folder.join("verschwunden"));
    ensure_handle_allowed(&outside).unwrap();
    ensure_handle_allowed(&inside).unwrap();
}
