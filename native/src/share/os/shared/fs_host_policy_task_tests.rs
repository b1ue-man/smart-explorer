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
