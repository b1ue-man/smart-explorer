use super::{installation_in, Installation};

#[cfg(any(target_os = "linux", windows))]
struct RestorePermissions(Vec<(std::path::PathBuf, std::fs::Permissions)>);
#[cfg(any(target_os = "linux", windows))]
impl Drop for RestorePermissions {
    fn drop(&mut self) {
        for (path, permissions) in &self.0 {
            let _ = std::fs::set_permissions(path, permissions.clone());
        }
    }
}
#[cfg(any(target_os = "linux", windows))]
fn readonly(paths: &[std::path::PathBuf]) -> RestorePermissions {
    let restore = RestorePermissions(
        paths
            .iter()
            .map(|path| (path.clone(), std::fs::metadata(path).unwrap().permissions()))
            .collect(),
    );
    for path in paths.iter().rev() {
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(path, permissions).unwrap();
    }
    restore
}

#[test]
fn cli_task_installation_detects_terminal_only_and_desktop() {
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("se");
    std::fs::write(&cli, b"se").unwrap();
    assert_eq!(
        installation_in(dir.path(), cli.clone()),
        Installation::TerminalOnly { cli: cli.clone() }
    );
    let app = dir.path().join(super::os::installed_app_names()[0]);
    std::fs::write(&app, b"app").unwrap();
    let desktop = installation_in(dir.path(), cli.clone());
    assert_eq!(desktop.cli(), cli.as_path());
    assert_eq!(desktop, Installation::Desktop { cli, app });
}

#[cfg(target_os = "linux")]
#[test]
fn batches_p13_legacy_profile_generation_serializes_with_terminal_updates() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("se");
    std::fs::write(&cli, b"local CLI").unwrap();
    let profile = super::lock_cli_for_integration(&cli).unwrap();
    let second = super::lock_cli_for_integration(&cli).unwrap();
    assert!(super::UpdateLock::acquire(&cli).is_err());
    assert!(!root.path().join("se.update-lock").exists());
    drop(profile);
    assert!(super::UpdateLock::acquire(&cli).is_err());
    drop(second);
    let update = super::UpdateLock::acquire(&cli).unwrap();
    assert!(super::lock_cli_for_integration(&cli).is_err());
    drop(update);
    drop(super::lock_cli_for_integration(&cli).unwrap());
    assert!(!root.path().join("se.update-lock").exists());
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn batches_p13_legacy_profiles_read_historical_update_state_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("se");
    let state = root.path().join("se.update-lock");
    std::fs::write(&cli, b"installed CLI").unwrap();
    for (raw, prefix) in [
        (
            std::process::id().to_string(),
            "integration_update_in_progress",
        ),
        ("invalid".into(), "integration_update_state_unknown"),
        ("0".into(), "integration_update_state_unknown"),
    ] {
        std::fs::write(&state, &raw).unwrap();
        assert!(super::lock_cli_for_integration(&cli)
            .err()
            .unwrap()
            .starts_with(prefix));
        assert_eq!(std::fs::read_to_string(&state).unwrap(), raw);
    }
    std::fs::remove_file(&state).unwrap();
    let profile = super::lock_cli_for_integration(&cli).unwrap();
    let owner = std::process::id().to_string();
    std::fs::write(&state, &owner).unwrap();
    assert!(profile
        .verify_current()
        .unwrap_err()
        .starts_with("integration_update_in_progress"));
    drop(profile);
    assert_eq!(std::fs::read_to_string(&state).unwrap(), owner);
    // Case "no installation writes": a read-only installation takes two locks without writing into it.
    {
        let root = tempfile::tempdir().unwrap();
        let cli = root.path().join("se");
        std::fs::write(&cli, b"read-only installed CLI").unwrap();
        let paths = [root.path().to_path_buf(), cli.clone()];
        let _permissions = readonly(&paths);
        let first = super::lock_cli_for_integration(&cli).unwrap();
        let second = super::lock_cli_for_integration(&cli).unwrap();
        first.verify_current().unwrap();
        second.verify_current().unwrap();
        let names = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, [std::ffi::OsString::from("se")], "no writes");
        let installed = std::fs::read(&cli).unwrap();
        assert_eq!(installed, b"read-only installed CLI", "no writes");
        assert!(
            std::fs::metadata(&cli).unwrap().permissions().readonly(),
            "no installation writes"
        );
        assert!(
            std::fs::metadata(root.path())
                .unwrap()
                .permissions()
                .readonly(),
            "no installation writes"
        );
        drop(first);
        drop(second);
        assert!(!root.path().join("se.update-lock").exists(), "no writes");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn batches_p13_legacy_profiles_preserve_stale_update_lock_in_readonly_installation() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("se");
    let state = root.path().join("se.update-lock");
    std::fs::write(&cli, b"installed CLI").unwrap();
    assert!(!super::os::process_alive(u32::MAX));
    let owner = u32::MAX.to_string();
    std::fs::write(&state, &owner).unwrap();
    let permissions = readonly(&[root.path().to_path_buf(), cli.clone(), state.clone()]);
    drop(super::lock_cli_for_integration(&cli).unwrap());
    assert_eq!(std::fs::read_to_string(&state).unwrap(), owner);
    drop(permissions);
    let update = super::UpdateLock::acquire(&cli).unwrap();
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        std::process::id().to_string()
    );
    drop(update);
    assert!(!state.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn batches_p13_legacy_profiles_reject_same_byte_image_replacement() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("se");
    let replacement = root.path().join("replacement");
    std::fs::write(&cli, b"same CLI bytes").unwrap();
    let profile = super::lock_cli_for_integration(&cli).unwrap();
    std::fs::write(&replacement, b"same CLI bytes").unwrap();
    // An uncooperative rename does not inherit the old file object's lease.
    std::fs::rename(&replacement, &cli).unwrap();
    assert!(profile
        .verify_current()
        .unwrap_err()
        .starts_with("integration_generator_replaced"));
    drop(profile);
    drop(super::lock_cli_for_integration(&cli).unwrap());
}

#[cfg(windows)]
#[test]
fn batches_p13_legacy_windows_profile_pins_cli_without_sidecar() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("se.exe");
    let moved = root.path().join("moved.exe");
    std::fs::write(&cli, b"installed CLI").unwrap();
    let profile = super::lock_cli_for_integration(&cli).unwrap();
    assert!(std::fs::OpenOptions::new().write(true).open(&cli).is_err());
    assert!(std::fs::rename(&cli, &moved).is_err());
    assert!(!root.path().join("se.exe.update-lock").exists());
    profile.verify_current().unwrap();
    drop(profile);
    std::fs::rename(&cli, &moved).unwrap();
    assert_eq!(std::fs::read(&moved).unwrap(), b"installed CLI");
}
