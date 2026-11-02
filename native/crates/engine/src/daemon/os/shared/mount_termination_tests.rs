//! Signals of the end of vault drives (DRV-M7, stage drive-vault; S2-V1,
//! S2-V6, C3.3): every end is classified and reaches the vault side, a lock
//! keeps the drive with its sealed changes for the next unlock, and a start
//! at a locked vault fails with the code `vault_locked` (a registered drive
//! waits for the unlock instead).
use std::path::PathBuf;

use super::super::{registered_entry, start_cache, MountManager};
use super::{after_exit, is_vault_locked, reported, terminated, waits_for_unlock, MountEndReason};
use crate::mount::{
    BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountRecovery, MountSource,
    MountStatus,
};

const SITE: &str = "0123456789abcdef0123456789abcdef";

fn vault_config(id: &str) -> MountConfig {
    MountConfig::new(
        MountId::parse(id).unwrap(),
        MountSource::Vault {
            site_id: SITE.into(),
        },
        DriveSelection::Automatic,
        MountMode::ReadWrite,
        "Privat",
    )
    .unwrap()
}

#[test]
fn drive_task_vault_unexpected_end_reported() {
    let mut entry = registered_entry(
        vault_config("vault-end"),
        MountRecovery::Clean,
        String::new(),
    );
    let folder = PathBuf::from("/run/user/1000/smart-explorer/Tresore/Privat");
    entry.status = MountStatus::MountedAt {
        path: folder.clone(),
    };
    // A host that died while mounted.
    let end = terminated(&entry, false).unwrap();
    assert_eq!(end.reason, MountEndReason::HostExit);
    assert_eq!(
        (end.site.as_str(), end.mount_id.as_str()),
        (SITE, "vault-end")
    );
    // Ejected outside se: the host reported `Unmounted` and ended cleanly.
    entry.status = MountStatus::Unmounted;
    assert_eq!(
        terminated(&entry, true).unwrap().reason,
        MountEndReason::ExternalUnmount
    );
    // The daemon asked it to stop.
    entry.stop_requested = true;
    assert_eq!(
        terminated(&entry, true).unwrap().reason,
        MountEndReason::UserUnmount
    );
    // The lock of the vault ended it.
    entry.lock_end = true;
    assert_eq!(
        terminated(&entry, true).unwrap().reason,
        MountEndReason::Lock
    );
    // Drives without a vault have no such end.
    let plain = MountConfig::new(
        MountId::parse("plain").unwrap(),
        MountSource::GoogleDrive {
            account: "cloud:gdrive".into(),
            root: BackendRoot::parse("/").unwrap(),
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "Drive",
    )
    .unwrap();
    let plain = registered_entry(plain, MountRecovery::Clean, String::new());
    assert!(terminated(&plain, false).is_none());

    // The unexpected end reaches the vault side with site, drive and
    // folder; the mounted folder leaves the plaintext boundary.
    let mut entry = registered_entry(
        vault_config("vault-crash"),
        MountRecovery::Clean,
        String::new(),
    );
    let status = MountStatus::MountedAt {
        path: folder.clone(),
    };
    reported(&entry.config, &status);
    let published = (Some(SITE.to_string()), folder.clone());
    assert!(crate::vault::vault_mount_roots().contains(&published));
    entry.status = MountStatus::Failed {
        detail: "the FUSE session ended without a requested unmount".into(),
    };
    let end = terminated(&entry, false).unwrap();
    let mut told = Vec::new();
    after_exit(&mut entry, end, |end, place| told.push((end, place)));
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].0.reason, MountEndReason::HostExit);
    assert_eq!(told[0].0.mount_id, "vault-crash");
    assert!(!crate::vault::vault_mount_roots().contains(&published));
    // The failure text stays the host's: an unexpected end is reported.
    assert!(matches!(&entry.status, MountStatus::Failed { detail } if detail.contains("FUSE")));

    // A lock end keeps the drive waiting, with its sealed changes counted.
    let mut locked = registered_entry(
        vault_config("vault-lock"),
        MountRecovery::Required,
        String::new(),
    );
    locked.lock_end = true;
    locked.lock_pending = Some(2);
    let end = terminated(&locked, true).unwrap();
    after_exit(&mut locked, end, |_, _| {});
    assert_eq!(
        locked.recovery,
        MountRecovery::RequiresUnlock { pending: 2 }
    );
    assert!(!locked.lock_end);
    match &locked.status {
        MountStatus::Failed { detail } => {
            assert!(is_vault_locked(detail), "{detail}");
            assert!(detail.contains("2 versiegelte"), "{detail}");
        }
        other => panic!("unexpected status {other:?}"),
    }
    assert!(waits_for_unlock(&locked, SITE));
    assert!(!waits_for_unlock(
        &locked,
        "ffffffffffffffffffffffffffffffff"
    ));
}

#[test]
fn drive_task_vault_locked_start_fails_vault_locked() {
    let manager = MountManager::from_registry(Ok(Vec::new()), |_| Vec::new());
    let cause = format!("vault_locked: Tresor Privat ist gesperrt: se vault unlock {SITE}");

    // A new drive at a locked vault: the start fails with the code.
    start_cache::insert(
        &manager,
        "vault-new",
        vault_config("vault-new"),
        false,
        None,
    )
    .unwrap();
    manager
        .state_guard()
        .unwrap()
        .get_mut("vault-new")
        .unwrap()
        .recovery = MountRecovery::Clean;
    let snapshot = manager
        .defer_until_unlock("vault-new", cause.clone(), Some(0))
        .unwrap();
    match &snapshot.status {
        MountStatus::Failed { detail } => {
            assert!(is_vault_locked(detail), "{detail}");
            assert!(!detail.contains("nach dem Entsperren"), "{detail}");
        }
        other => panic!("unexpected status {other:?}"),
    }
    assert_eq!(snapshot.recovery, MountRecovery::Clean);

    // A registered drive with sealed changes waits for the unlock.
    start_cache::insert(
        &manager,
        "vault-kept",
        vault_config("vault-kept"),
        true,
        None,
    )
    .unwrap();
    let snapshot = manager
        .defer_until_unlock("vault-kept", cause, Some(2))
        .unwrap();
    assert_eq!(
        snapshot.recovery,
        MountRecovery::RequiresUnlock { pending: 2 }
    );
    match &snapshot.status {
        MountStatus::Failed { detail } => {
            assert!(is_vault_locked(detail), "{detail}");
            assert!(
                detail.ends_with("nach dem Entsperren eingebunden"),
                "{detail}"
            );
        }
        other => panic!("unexpected status {other:?}"),
    }
    let state = manager.state_guard().unwrap();
    assert!(waits_for_unlock(&state["vault-kept"], SITE));
    assert!(!waits_for_unlock(&state["vault-new"], SITE));
    assert!(!is_vault_locked("vault_lockedness"));
}

#[cfg(target_os = "linux")]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

/// Stage drive-vault with the fixture `runtime-tmpfs` (S2-V7, decision S3
/// OE2): the vault place lies on tmpfs, is locked `1500` outside a mount and
/// open `1700` for it, the sticky bit always set.
#[cfg(target_os = "linux")]
#[test]
fn drive_task_vault_linux_mountpoint_tmpfs_sticky_locked() {
    use crate::mount::os::linux_os::mount_point::{
        lock, open_for_mount, place_kind, prepare, vaults_root, PlaceKind,
    };
    let root = vaults_root().expect("XDG_RUNTIME_DIR (fixture runtime-tmpfs)");
    let place = root.join(format!("U14 Tresor {}", std::process::id()));
    assert_eq!(place_kind(&place), PlaceKind::Vault);
    prepare(&place, PlaceKind::Vault).unwrap();
    assert_eq!(mode(&place), 0o1500);
    assert!(
        std::fs::write(place.join("x"), b"x").is_err() || unsafe { libc::geteuid() } == 0,
        "the locked vault place took a file"
    );
    open_for_mount(&place, PlaceKind::Vault).unwrap();
    assert_eq!(mode(&place), 0o1700);
    lock(&place, PlaceKind::Vault).unwrap();
    assert_eq!(mode(&place), 0o1500);
    // A vault drive configured there passes the start check.
    let config = vault_config("vault-place")
        .with_location_directory(Some(place.clone()))
        .unwrap();
    assert_eq!(super::vault_place_refusal(&config), None);
    open_for_mount(&place, PlaceKind::Vault).unwrap();
    std::fs::remove_dir(&place).unwrap();
}

/// Stage drive-vault: a vault folder off tmpfs is refused by the host, and
/// a vault drive configured outside the vault place by the start.
#[cfg(target_os = "linux")]
#[test]
fn drive_task_vault_linux_refuses_non_tmpfs() {
    use crate::mount::os::linux_os::mount_point::{prepare, PlaceKind};
    // The checkout lies on a disk file system, never on tmpfs.
    let base = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let folder = base.path().join("Tresor");
    let error = prepare(&folder, PlaceKind::Vault).unwrap_err();
    assert!(error.to_string().contains("tmpfs"), "{error}");
    let config = vault_config("vault-disk")
        .with_location_directory(Some(folder))
        .unwrap();
    let refusal = super::vault_place_refusal(&config).expect("a disk folder was accepted");
    assert!(refusal.contains("Arbeitsspeicher"), "{refusal}");
}
