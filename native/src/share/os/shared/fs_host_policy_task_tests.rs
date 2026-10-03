use std::io;

use super::{
    export_config::ExportAccess,
    fs_host_policy::TargetPolicy,
    fs_local_paths::{secure_local_target, to_os_path},
    fs_path_adapter::create_directory_link,
};

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
    let root = crate::support_dirs::app_data_dir().to_string_lossy().replace('\\', "/");
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
