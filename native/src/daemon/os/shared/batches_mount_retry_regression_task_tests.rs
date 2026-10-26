//! RA-07 fixtures for the one remote review suite. No provider account or
//! mount host is started: the real retry coordinator receives a deterministic
//! preflight failure, and the real entry/stream handoff owns the transitions.
use super::{snapshot, start_cache, MountManager, StartupLease};
use crate::mount::{
    BackendRoot, DriveLetter, DriveSelection, MountConfig, MountId, MountMode,
    MountRecovery, MountRootSecurity, MountSnapshot, MountSource, MountStatus, PeerMountTarget,
};
use std::sync::Arc;

fn config(source: MountSource) -> MountConfig {
    MountConfig::new(
        MountId::parse("retry-vault").unwrap(),
        source,
        DriveSelection::Letter(DriveLetter::parse('R').unwrap()),
        MountMode::ReadWrite,
        "Retry Tresor",
    )
    .unwrap()
    .with_location_directory(Some(std::env::temp_dir().join("retry-vault-place")))
    .unwrap()
    .with_auto_remount(true)
}

fn vault_config() -> MountConfig {
    config(MountSource::Vault { site_id: "retry-site".into() })
}

fn manager(config: MountConfig, recovery: MountRecovery) -> MountManager {
    MountManager::from_registry(Ok(vec![config]), |_| vec![recovery])
}

fn only_mount(manager: &MountManager) -> MountSnapshot {
    let mounts = manager.list().unwrap();
    assert_eq!(mounts.len(), 1);
    mounts.into_iter().next().unwrap()
}

fn restart(
    manager: &MountManager,
    config: MountConfig,
    recorded: bool,
    lease: Option<Arc<dyn Send + Sync>>,
) -> Result<MountSnapshot, String> {
    let key = config.id.as_str().to_string();
    start_cache::restart(manager, &key, config, recorded, lease)?;
    let state = manager.state_guard()?;
    Ok(snapshot(state.get(&key).unwrap()))
}

#[test]
fn batches_task_vault_retry_preflight_failure_keeps_list_warning_and_same_id() {
    for recovery in [
        MountRecovery::Required,
        MountRecovery::Unknown,
        MountRecovery::RequiresUnlock { pending: 7 },
    ] {
        for mode in [MountMode::ReadOnly, MountMode::ReadWrite] {
            for security in [MountRootSecurity::Enforced, MountRootSecurity::Trusted] {
                let mut config = vault_config().with_root_security(security);
                config.mode = mode;
                let manager = manager(config.clone(), recovery);
                let before = only_mount(&manager);
                // List remains available while the remote policy lookup is
                // failing, as well as after its error has been returned.
                for _ in 0..2 {
                    let error = manager.retry_with(&config.id, |manager, received, recorded| {
                        assert_eq!(received, config);
                        assert!(recorded);
                        assert_eq!(only_mount(manager), before);
                        Err("Vault-Speicher vorübergehend nicht erreichbar".into())
                    }).unwrap_err();
                    assert_eq!(error, "Vault-Speicher vorübergehend nicht erreichbar");
                    manager.tick();
                    assert_eq!(only_mount(&manager), before);
                    assert!(before.recovery_required_compat);
                }
                // Provider recovery may start the SAME ID. No new creation,
                // reactivation or daemon restart is needed to admit it.
                let started = manager.retry_with(&config.id, |manager, received, recorded| {
                    assert_eq!(only_mount(manager), before);
                    restart(manager, received, recorded, None)
                }).unwrap();
                assert_eq!(started.config, config);
                assert_eq!(started.status, MountStatus::Mounting);
                assert_eq!(started.recovery, recovery);
                assert!(started.recovery_required_compat);
                assert_eq!(only_mount(&manager), started);
            }
        }
    }
}

#[test]
fn batches_task_vault_retry_failure_preserves_unlock_code_tokens_and_lease() {
    let config = vault_config();
    let manager = manager(config.clone(), MountRecovery::Unknown);
    let old_owner = Arc::new(());
    let old_weak = Arc::downgrade(&old_owner);
    let (control, receiver) = std::sync::mpsc::sync_channel(1);
    {
        let mut state = manager.state_guard().unwrap();
        let entry = state.get_mut(config.id.as_str()).unwrap();
        entry.status = super::super::termination::waiting_status("retry-site", None);
        entry.startup_lease = Some(StartupLease::new(old_owner));
        entry.launch_token = Some("old-launch".into());
        entry.session_token = Some("old-session".into());
        entry.backend_token = Some("old-backend".into());
        entry.control = Some(control);
    }
    let before = only_mount(&manager);
    let attempted = Arc::new(());
    let attempted_weak = Arc::downgrade(&attempted);
    manager.retry_with(&config.id, |_, _, _| {
        let _attempt_lease: Arc<dyn Send + Sync> = attempted;
        Err("Vaultidentität ist momentan nicht lesbar".into())
    }).unwrap_err();
    assert!(attempted_weak.upgrade().is_none());
    assert!(old_weak.upgrade().is_some());
    assert_eq!(only_mount(&manager), before);
    assert!(matches!(&before.status, MountStatus::Failed { detail }
        if super::super::termination::is_vault_locked(detail)));
    manager.check_launch_token(&config.id, "old-launch").unwrap();
    manager.check_session_token(&config.id, "old-session").unwrap();
    manager.check_backend_token(&config.id, "old-backend").unwrap();
    manager.state_guard().unwrap().get(config.id.as_str()).unwrap()
        .control.as_ref().unwrap().try_send(()).unwrap();
    receiver.try_recv().unwrap();

    let next_owner = Arc::new(());
    let next_weak = Arc::downgrade(&next_owner);
    manager.retry_with(&config.id, |manager, received, recorded| {
        restart(manager, received, recorded, Some(next_owner))
    }).unwrap();
    assert!(old_weak.upgrade().is_none());
    assert!(next_weak.upgrade().is_some());
    assert!(manager.check_launch_token(&config.id, "old-launch").is_err());
    assert!(manager.check_session_token(&config.id, "old-session").is_err());
    assert!(manager.check_backend_token(&config.id, "old-backend").is_err());
    // The existing authenticated-mounted boundary releases the new fence.
    manager.state_guard().unwrap().get(config.id.as_str()).unwrap()
        .startup_lease.as_ref().unwrap().mounted();
    assert!(next_weak.upgrade().is_none());
}

#[test]
fn batches_task_vault_retry_existing_and_late_backend_stream_keep_owner() {
    for late in [false, true] {
        let config = vault_config();
        let manager = manager(config.clone(), MountRecovery::Required);
        let backend = crate::mount::range_read_task_tests::RangeBackend::new(16);
        {
            let mut state = manager.state_guard().unwrap();
            let entry = state.get_mut(config.id.as_str()).unwrap();
            entry.backend = Some(backend);
            entry.backend_token = Some("backend-generation".into());
            entry.host_terminal_failure = true;
        }
        let before = only_mount(&manager);
        if late {
            manager.retry_with(&config.id, |manager, received, recorded| {
                let (_backend, serving) = manager.take_backend(&config.id, "backend-generation")?;
                let error = restart(manager, received, recorded, None).unwrap_err();
                assert!(error.contains("Remote-Aufruf"));
                assert_eq!(only_mount(manager), before);
                assert!(manager.state_guard()?.get(config.id.as_str()).unwrap().backend_stream_active);
                drop(serving);
                Err(error)
            }).unwrap_err();
        } else {
            let (_backend, serving) = manager.take_backend(&config.id, "backend-generation").unwrap();
            let error = manager.retry_with(&config.id, |_, _, _| {
                panic!("an owned backend stream must prevent a new attempt")
            }).unwrap_err();
            assert!(error.contains("Remote-Aufruf"));
            assert_eq!(only_mount(&manager), before);
            assert!(manager.state_guard().unwrap().get(config.id.as_str()).unwrap().backend_stream_active);
            drop(serving);
        }
        assert_eq!(only_mount(&manager), before);
        manager.retry_with(&config.id, |manager, received, recorded| {
            restart(manager, received, recorded, None)
        }).unwrap();
        assert_eq!(only_mount(&manager).recovery, MountRecovery::Required);
    }
}

#[test]
fn batches_task_vault_retry_new_start_cannot_replace_and_missing_retry_cannot_create() {
    let config = vault_config();
    let manager = manager(config.clone(), MountRecovery::Required);
    let before = only_mount(&manager);
    let error = start_cache::insert(&manager, config.id.as_str(), config.clone(), false, None)
        .unwrap_err();
    assert!(error.contains("bereits verwaltet"));
    assert_eq!(only_mount(&manager), before);
    let missing = MountId::parse("missing-vault").unwrap();
    assert!(manager.retry_with(&missing, |_, _, _| {
        panic!("an unknown ID must not create a generation")
    }).unwrap_err().contains("nicht verwaltet"));
    let mut missing_config = config;
    missing_config.id = missing;
    assert!(start_cache::restart(&manager, missing_config.id.as_str(), missing_config.clone(), false, None)
        .unwrap_err().contains("nicht verwaltet"));
    assert_eq!(only_mount(&manager), before);
}

#[test]
fn batches_task_vault_retry_clean_preflight_error_retains_failed_action() {
    let config = vault_config().with_auto_remount(false);
    let manager = manager(config.clone(), MountRecovery::Clean);
    // This entry has no persisted owner to remove. The fixture never reads or
    // writes the runner's default registry, even for the clean Stop path.
    manager.state_guard().unwrap().get_mut(config.id.as_str()).unwrap().registry_recorded = false;
    manager.retry_with(&config.id, |manager, _, recorded| {
        assert!(!recorded);
        assert_eq!(only_mount(manager).status, MountStatus::Unmounted);
        Err("Vault-Policy konnte nicht geprüft werden".into())
    }).unwrap_err();
    let retained = only_mount(&manager);
    assert_eq!(retained.config, config);
    assert_eq!(retained.recovery, MountRecovery::Clean);
    assert!(!retained.recovery_required_compat);
    assert!(matches!(&retained.status, MountStatus::Failed { detail }
        if detail == "Vault-Policy konnte nicht geprüft werden"));
    manager.tick();
    assert_eq!(only_mount(&manager), retained);
    manager.retry_with(&config.id, |manager, received, recorded| {
        assert!(!recorded);
        restart(manager, received, recorded, None)
    }).unwrap();
    assert_eq!(only_mount(&manager).status, MountStatus::Mounting);
}

#[test]
fn batches_task_vault_retry_shared_sources_keep_endpoint_identity() {
    let root = BackendRoot::parse("/same%20literal").unwrap();
    let sources = [
        MountSource::SavedRemote { account: "sftp:first".into(), root: root.clone() },
        MountSource::SavedRemote { account: "sftp:second".into(), root: root.clone() },
        MountSource::SavedRemote { account: "ftp:saved".into(), root: root.clone() },
        MountSource::SavedRemote { account: "webdav:saved".into(), root: root.clone() },
        MountSource::GoogleDrive { account: "cloud:gdrive".into(), root: root.clone() },
        MountSource::Peer { target: PeerMountTarget::Direct { contact_id: "direct-peer".into() }, root: root.clone() },
        MountSource::Peer { target: PeerMountTarget::RoomDevice { room_id: "room".into(), device_id: "room-peer".into() }, root },
        MountSource::Devices,
    ];
    for source in sources {
        let config = config(source);
        let manager = manager(config.clone(), MountRecovery::Required);
        let before = only_mount(&manager);
        manager.retry_with(&config.id, |manager, received, recorded| {
            assert_eq!(received, config);
            assert!(recorded);
            assert_eq!(only_mount(manager), before);
            Err("source unavailable".into())
        }).unwrap_err();
        assert_eq!(only_mount(&manager), before);
        let started = manager.retry_with(&config.id, |manager, received, recorded| {
            restart(manager, received, recorded, None)
        }).unwrap();
        assert_eq!(started.config, config);
        assert_eq!(started.recovery, MountRecovery::Required);
    }
}
