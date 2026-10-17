//! Acceptance signals of DRV-M3 for the restore at login (plan 2026-10-17,
//! strand S2): a worker restart is a new drive table built from the
//! persisted registry (`MountManager::from_registry`), as the worker of every
//! start builds it. The drive host itself is replaced by a start that only
//! records the call (a Dokany or FUSE mount is not part of a unit test).
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::super::super::mount_registry;
use super::super::{start_cache, MountManager, RESTART_DETAIL};
use super::{Retention, DEFERRED_DETAIL, RESTORE_FAILED_PREFIX};
use crate::mount::{
    BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountRecovery, MountSnapshot,
    MountSource, MountStatus, PeerMountTarget,
};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "se_drive_restore_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn saved(id: &str) -> MountConfig {
    MountConfig::new(
        MountId::parse(id).unwrap(),
        MountSource::SavedRemote {
            account: "sftp:prod".into(),
            root: BackendRoot::parse("/srv/data").unwrap(),
        },
        DriveSelection::Automatic,
        MountMode::ReadWrite,
        id,
    )
    .unwrap()
}

fn peer(id: &str) -> MountConfig {
    MountConfig::new(
        MountId::parse(id).unwrap(),
        MountSource::Peer {
            target: PeerMountTarget::Direct {
                contact_id: "contact-1".into(),
            },
            root: BackendRoot::parse("/Fotos").unwrap(),
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        id,
    )
    .unwrap()
}

fn clean_audit(configs: &[MountConfig]) -> Vec<MountRecovery> {
    vec![MountRecovery::Clean; configs.len()]
}

fn registry_bytes(dir: &Path) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    (
        std::fs::read(dir.join("registry.json")).ok(),
        std::fs::read(dir.join("registry-2.json")).ok(),
    )
}

/// What `start_locked(…, true)` leaves before its host reports: a `Mounting`
/// entry whose registry record is kept.
fn start_like_host(manager: &MountManager, config: MountConfig) -> Result<MountSnapshot, String> {
    let key = config.id.as_str().to_string();
    start_cache::insert(manager, &key, config, true)?;
    status_of(manager, &key).ok_or_else(|| "started drive is missing".to_string())
}

fn status_of(manager: &MountManager, id: &str) -> Option<MountSnapshot> {
    manager
        .list()
        .unwrap()
        .into_iter()
        .find(|mount| mount.config.id.as_str() == id)
}

fn detail(snapshot: &MountSnapshot) -> &str {
    match &snapshot.status {
        MountStatus::Failed { detail } => detail,
        other => panic!("unexpected status {other:?}"),
    }
}

#[test]
fn drive_task_existing_drive_after_restart_unchanged() {
    let dir = temp_dir("existing");
    // Written by v0.5.182: no option of this batch.
    let fixture = concat!(
        r#"{"version":1,"mounts":[{"id":"existing-drive","source":{"SavedRemote":"#,
        r#"{"account":"sftp:prod","root":"/srv/data"}},"drive":{"Letter":"M"},"#,
        r#""mode":"ReadWrite","root_security":"enforced","metadata":{"preload_depth":2},"#,
        r#""cache":{"retained_mib":500},"runtime_preference":"auto","label":"Projekte"}]}"#
    );
    std::fs::write(dir.join("registry.json"), fixture).unwrap();
    let manager = MountManager::from_registry(mount_registry::load_in(&dir), clean_audit);
    let before = manager.list().unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(detail(&before[0]), RESTART_DETAIL);
    assert!(!before[0].config.auto_remount);
    assert!(before[0].config.location.is_none());
    assert!(!before[0].config.restores_at_login());
    // The definition the GUI and `se drive list --json` see is the stored one.
    let config = serde_json::to_string(&before[0].config).unwrap();
    assert!(fixture.contains(&config), "{config}");
    // The restore leaves it for Retry, as every worker of v0.5.182 did.
    let mut started = Vec::new();
    let restored = manager.restore_with(true, |_, config| {
        started.push(config.id);
        Err("no start expected".into())
    });
    assert!(restored.is_empty());
    assert!(started.is_empty());
    assert_eq!(manager.list().unwrap(), before);
    assert_eq!(
        registry_bytes(&dir),
        (Some(fixture.as_bytes().to_vec()), None)
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn drive_task_auto_remount_after_daemon_restart() {
    let dir = temp_dir("restart");
    // First worker: one drive mounted with --auto-remount, one plain drive,
    // one Share drive with the option, one drive switched on by `se drive set`.
    let login = saved("login-drive").with_auto_remount(true);
    let plain = saved("plain-drive");
    let share = peer("share-drive").with_auto_remount(true);
    let switched = saved("switched-drive");
    for config in [&login, &plain, &share, &switched] {
        mount_registry::upsert_in(&dir, config).unwrap();
    }
    let first = MountManager::from_registry(mount_registry::load_in(&dir), clean_audit);
    let set = first
        .set_options_with(&switched.id, true, |config| {
            mount_registry::upsert_in(&dir, config).map(|_| ())
        })
        .unwrap();
    assert!(set.config.auto_remount);
    let written = registry_bytes(&dir);
    drop(first);

    // Restart: a new table from the persisted registry. A worker started by
    // the login entry without sync autostart is restricted: no Share yet.
    let worker = MountManager::from_registry(mount_registry::load_in(&dir), clean_audit);
    let mut started = Vec::new();
    let restored = worker.restore_with(false, |manager, config| {
        started.push(config.clone());
        start_like_host(manager, config)
    });
    let ids: Vec<_> = started.iter().map(|config| config.id.as_str()).collect();
    assert_eq!(ids, ["login-drive", "switched-drive"]);
    assert_eq!(started[0], login);
    assert_eq!(started[1], switched.clone().with_auto_remount(true));
    assert_eq!(restored.len(), 2);
    assert!(restored
        .iter()
        .all(|mount| mount.status == MountStatus::Mounting));
    assert_eq!(
        detail(&status_of(&worker, "plain-drive").unwrap()),
        RESTART_DETAIL
    );
    assert_eq!(
        detail(&status_of(&worker, "share-drive").unwrap()),
        DEFERRED_DETAIL
    );

    // The promotion (first request of the app or of se) mounts the Share
    // drive; running drives are not started twice.
    started.clear();
    worker.restore_with(true, |manager, config| {
        started.push(config.clone());
        start_like_host(manager, config)
    });
    assert_eq!(started, [share.clone()]);
    assert_eq!(
        status_of(&worker, "share-drive").unwrap().status,
        MountStatus::Mounting
    );
    // Nothing rewrote the registry.
    assert_eq!(registry_bytes(&dir), written);

    // A worker stop keeps exactly the records of drives that restore.
    let mut retention = Retention::default();
    let state = worker.state_guard().unwrap();
    assert!(!retention.keeps(&state["login-drive"]));
    retention.begin_shutdown();
    assert!(retention.keeps(&state["login-drive"]));
    assert!(!retention.keeps(&state["plain-drive"]));
    drop(state);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn drive_task_restore_runs_after_recovery_audit() {
    let dir = temp_dir("audit");
    let pending = saved("pending-drive").with_auto_remount(true);
    let clean = saved("clean-drive").with_auto_remount(true);
    let manual = saved("manual-drive");
    for config in [&pending, &clean, &manual] {
        mount_registry::upsert_in(&dir, config).unwrap();
    }
    let log = RefCell::new(Vec::<String>::new());
    let worker = MountManager::from_registry(mount_registry::load_in(&dir), |configs| {
        log.borrow_mut().push(format!("audit {}", configs.len()));
        configs
            .iter()
            .map(|config| match config.id.as_str() {
                "clean-drive" => MountRecovery::Clean,
                _ => MountRecovery::Required,
            })
            .collect()
    });
    // The audit result is in place before the restore picks its drives.
    let audited = worker.list().unwrap();
    let recovery = |id: &str| {
        audited
            .iter()
            .find(|mount| mount.config.id.as_str() == id)
            .unwrap()
            .recovery
    };
    assert_eq!(recovery("pending-drive"), MountRecovery::Required);
    assert_eq!(recovery("clean-drive"), MountRecovery::Clean);
    worker.restore_with(true, |manager, config| {
        log.borrow_mut().push(format!("start {}", config.id));
        if config.id.as_str() == "pending-drive" {
            return Err("remote unreachable".into());
        }
        start_like_host(manager, config)
    });
    assert_eq!(
        *log.borrow(),
        ["audit 3", "start clean-drive", "start pending-drive"]
    );
    // A restore that fails keeps the drive, its option and the evidence of
    // its journal for the next attempt.
    let failed = status_of(&worker, "pending-drive").unwrap();
    assert_eq!(
        detail(&failed),
        format!("{RESTORE_FAILED_PREFIX}remote unreachable")
    );
    assert_eq!(failed.recovery, MountRecovery::Required);
    assert!(failed.config.auto_remount);
    // A drive with pending changes but without the option is never started
    // by itself; it waits for Retry as before.
    let waiting = status_of(&worker, "manual-drive").unwrap();
    assert_eq!(detail(&waiting), RESTART_DETAIL);
    assert_eq!(waiting.recovery, MountRecovery::Required);
    std::fs::remove_dir_all(dir).ok();
}
