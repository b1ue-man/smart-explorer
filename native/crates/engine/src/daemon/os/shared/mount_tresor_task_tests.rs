//! E35, M28 (1) (TODO `SEC-FS-VAULT-PROZESS`): the chain from the status a
//! drive host reports to the Share host's check. A vault drive that reports
//! `MountedAt` through `MountManager::update_status` is published to the
//! plaintext boundary, so no export may be added in it; it stays published
//! after `Unmounted` until the end of its host process is booked, and only
//! then an export there is admitted again.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::daemon::mount_manager::termination::{after_exit, terminated};
use crate::mount::{DriveSelection, MountConfig, MountId, MountMode, MountSource, MountStatus};
use crate::share::profile_edits::refuse_vault_exports;
use crate::share::{ShareProfiles, SharedRoot};
use crate::vfs::{BackendHandle, LocalBackend};

use super::MountManager;

const SITE: &str = "fedcba9876543210fedcba9876543210";

/// The drive table and the vault register are process-wide: the row and the
/// published root go again even when an assertion fails.
struct Cleanup {
    manager: MountManager,
    id: MountId,
    path: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.manager.forget_for_test(&self.id);
        crate::vault::withdraw_vault_mount(SITE, &self.path);
    }
}

/// Profiles that would add an export at `path`.
fn exporting(path: &Path) -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots.push(SharedRoot::new(
        "Tresor",
        path.to_string_lossy().into_owned(),
    ));
    profiles
}

#[test]
fn vault_task_mount_status_publishes_the_drive_until_the_host_ended() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tresor-laufwerk");
    std::fs::create_dir(&path).unwrap();
    let manager = MountManager::default();
    let id = MountId::new_random().unwrap();
    let cleanup = Cleanup {
        manager: manager.clone(),
        id: id.clone(),
        path: path.clone(),
    };
    let config = MountConfig::new(
        id.clone(),
        MountSource::Vault {
            site_id: SITE.into(),
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "Tresor-Probe",
    )
    .unwrap();
    let backend: BackendHandle = Arc::new(LocalBackend::new(&directory.path().to_string_lossy()));
    let launch_token = "c3".repeat(32);
    manager
        .arm_host_for_test(config, backend, &launch_token)
        .unwrap();
    let session_token = manager
        .grant_host(&id, &launch_token)
        .unwrap()
        .session_token;
    let profiles = exporting(&path);
    assert_eq!(crate::vault::vault_mount_of(&path), None);
    assert_eq!(refuse_vault_exports(&[], &profiles), Ok(()));

    // The host reports the mounted folder: the drive is private.
    manager
        .update_status(
            &id,
            &session_token,
            MountStatus::MountedAt { path: path.clone() },
            None,
        )
        .unwrap();
    assert_eq!(
        crate::vault::vault_mount_of(&path),
        Some(Some(SITE.to_string()))
    );
    assert!(refuse_vault_exports(&[], &profiles).is_err());

    // `Unmounted` alone does not withdraw it: the host has not ended yet.
    manager
        .update_status(&id, &session_token, MountStatus::Unmounted, None)
        .unwrap();
    assert!(refuse_vault_exports(&[], &profiles).is_err());

    // The end of the host process is booked: the drive leaves the boundary.
    {
        let mut state = manager.state_guard().unwrap();
        let entry = state.get_mut(id.as_str()).unwrap();
        let end = terminated(entry, true).expect("a vault drive ends with a reason");
        after_exit(entry, end, |_, _| {});
    }
    assert_eq!(crate::vault::vault_mount_of(&path), None);
    assert_eq!(refuse_vault_exports(&[], &profiles), Ok(()));
    drop(cleanup);
}
