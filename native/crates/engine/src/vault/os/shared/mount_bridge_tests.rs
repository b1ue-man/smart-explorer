//! Signals of the bridge between the vault sessions and the drive manager
//! (VLT-M12, stage `vault_unit`, job linux-core: unit; S2-V1, S2-V4, S2-V6,
//! C3.3): a drive needs an unlocked vault, its end follows the consumer
//! rule, every unlock reaches the drive manager (which restores the drives
//! that wait and so retries their sealed changes), and the drive
//! participant runs before the masterkey is wiped.
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use crate::vault::core::lock_policy::{LockPlatform, LockReason};
use crate::vault::core::{locked_site, VaultSession};
use crate::vfs::{Backend, Scheme};

use super::mount_bridge::{MountVaultError, VaultDriveBridge};
use super::session::{ConsumerKind, LockParticipant, LockRequest, SessionRegister, SiteState};
use super::session_test_support::{site_fixture, test_register, wait_until, SiteFixture};
use super::spool_key_tests::{bridge_for, unlock};

const PASSWORD: &str = "Laufwerk-Passwort-1";

fn consumers(register: &SessionRegister, fixture: &SiteFixture) -> usize {
    register.status(&[fixture.record.clone()])[0].consumers
}

fn last_lock_reason(register: &SessionRegister, site: &str) -> Option<LockReason> {
    let (_, events) = register.events();
    events
        .iter()
        .rev()
        .find(|event| event.site == site && event.state == SiteState::Locked)
        .and_then(|event| event.reason)
}

#[test]
fn vault_task_mount_requires_unlock() {
    let fixture = site_fixture("Laufwerk", PASSWORD);
    let site = fixture.record.id.clone();
    let register = test_register(LockPlatform::Desktop);
    let bridge = bridge_for(&fixture, Arc::clone(&register));

    // Locked: no source, no consumer.
    let error = bridge
        .open_backend(&site)
        .err()
        .expect("a locked vault opened");
    assert_eq!(
        error,
        MountVaultError::Locked {
            site: site.clone(),
            label: fixture.record.display_label().to_string(),
        }
    );
    assert_eq!(error.code(), Some("vault_locked"));
    assert!(error
        .to_string()
        .contains(&format!("se vault unlock {site}")));
    let refused = bridge.started(&site, "drive-1", None).unwrap_err();
    assert_eq!(locked_site(&refused), Some(site.as_str()));
    assert_eq!(consumers(&register, &fixture), 0);
    let unknown = bridge.open_backend("0123456789abcdef0123456789abcdef");
    assert!(matches!(unknown, Err(MountVaultError::Failed(_))));

    // Unlocked: the source is the vault's cleartext view, the drive holds
    // the session.
    unlock(&register, &fixture, PASSWORD);
    let backend = bridge.open_backend(&site).ok().expect("vault opened");
    assert_eq!(backend.scheme(), Scheme::Vault);
    assert!(backend.list_dir("/").unwrap().is_empty());
    bridge.started(&site, "drive-1", None).unwrap();
    assert_eq!(consumers(&register, &fixture), 1);

    // After the lock nothing opens, and the open view fails with
    // `vault_locked` (never as an empty folder).
    let request = LockRequest::new(LockReason::Explicit, true);
    assert_eq!(register.lock(&site, request), Ok(true));
    assert!(matches!(
        bridge.open_backend(&site),
        Err(MountVaultError::Locked { .. })
    ));
    let error = backend.list_dir("/").unwrap_err();
    assert_eq!(locked_site(&error), Some(site.as_str()));
}

#[test]
fn vault_task_mount_end_reason_handling() {
    let fixture = site_fixture("Enden", PASSWORD);
    let site = fixture.record.id.clone();
    let register = test_register(LockPlatform::Desktop);
    let bridge = bridge_for(&fixture, Arc::clone(&register));
    unlock(&register, &fixture, PASSWORD);
    bridge.started(&site, "drive-a", None).unwrap();
    bridge.started(&site, "drive-b", None).unwrap();
    assert_eq!(consumers(&register, &fixture), 2);

    // user_unmount, external_unmount, host_exit: one drive ends while the
    // other runs, the vault stays unlocked.
    assert!(!bridge.ended(&site, "drive-a", None, false));
    assert!(register.session(&site).is_some());
    assert_eq!(consumers(&register, &fixture), 1);
    // lock: a drive that a lock ended never locks again.
    assert!(!bridge.ended(&site, "drive-b", None, true));
    assert!(register.session(&site).is_some());
    assert_eq!(consumers(&register, &fixture), 0);

    // An open view (GUI tab, CLI command) keeps the session after the last
    // drive ended.
    let open = register.consume(&site, ConsumerKind::Open).unwrap();
    bridge.started(&site, "drive-c", None).unwrap();
    assert!(!bridge.ended(&site, "drive-c", None, false));
    assert!(register.session(&site).is_some());
    drop(open);

    // The drive was the last consumer: its end locks the vault (mount_end).
    bridge.started(&site, "drive-d", None).unwrap();
    assert!(bridge.ended(&site, "drive-d", None, false));
    assert!(register.session(&site).is_none());
    assert_eq!(
        last_lock_reason(&register, &site),
        Some(LockReason::MountEnd)
    );
    // A late report of the same end changes nothing.
    assert!(!bridge.ended(&site, "drive-d", None, false));
}

#[test]
fn vault_task_unlock_triggers_pending_retry() {
    let fixture = site_fixture("Wiederholen", PASSWORD);
    let site = fixture.record.id.clone();
    let register = test_register(LockPlatform::Desktop);
    let bridge = bridge_for(&fixture, Arc::clone(&register));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let hook_seen = Arc::clone(&seen);
    bridge.watch(Arc::new(move |site: &str| {
        hook_seen.lock().unwrap().push(site.to_string())
    }));
    // A second watch is ignored: one restore per unlock.
    bridge.watch(Arc::new(|_: &str| panic!("second watcher called")));
    let count = || seen.lock().unwrap().len();

    unlock(&register, &fixture, PASSWORD);
    wait_until("Entsperren gemeldet", || count() == 1);
    assert_eq!(seen.lock().unwrap()[0], site);

    // A lock that unsaved changes refuse leaves the vault unlocked; the
    // drive manager hears it again and mounts what the lock ended.
    struct Refusing;
    impl LockParticipant for Refusing {
        fn prepare_lock(&self, _site: &str, _request: &LockRequest) -> Result<(), String> {
            Err("1 Änderung nicht gespeichert".into())
        }
    }
    let participant = register.add_participant(Arc::new(Refusing));
    let refused = register.lock(&site, LockRequest::new(LockReason::Explicit, false));
    assert!(refused.is_err());
    wait_until("abgelehnte Sperre gemeldet", || count() == 2);
    register.remove_participant(participant);

    // A lock is no unlock; the next unlock is reported again.
    let request = LockRequest::new(LockReason::Explicit, false);
    assert_eq!(register.lock(&site, request), Ok(true));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(count(), 2);
    unlock(&register, &fixture, PASSWORD);
    wait_until("zweites Entsperren gemeldet", || count() == 3);
}

/// The drive side of a lock: records whether the key was still there when
/// it ended its drive.
struct DriveParticipant {
    bridge: Weak<VaultDriveBridge>,
    session: Arc<VaultSession>,
    key_present: Mutex<Vec<bool>>,
}

impl LockParticipant for DriveParticipant {
    fn prepare_lock(&self, site: &str, _request: &LockRequest) -> Result<(), String> {
        self.key_present
            .lock()
            .unwrap()
            .push(!self.session.is_locked());
        if let Some(bridge) = self.bridge.upgrade() {
            // The drive unmounts inside the lock: never a second lock.
            assert!(!bridge.ended(site, "drive-1", None, true));
        }
        Ok(())
    }
}

#[test]
fn vault_task_lock_unmounts_before_zeroize() {
    let fixture = site_fixture("Reihenfolge", PASSWORD);
    let site = fixture.record.id.clone();
    let register = test_register(LockPlatform::Desktop);
    let bridge = bridge_for(&fixture, Arc::clone(&register));
    unlock(&register, &fixture, PASSWORD);
    let (session, _) = register.session(&site).unwrap();
    bridge.started(&site, "drive-1", None).unwrap();
    let participant = Arc::new(DriveParticipant {
        bridge: Arc::downgrade(&bridge),
        session: Arc::clone(&session),
        key_present: Mutex::new(Vec::new()),
    });
    register.add_participant(participant.clone());

    let request = LockRequest::new(LockReason::Sleep, false);
    assert_eq!(register.lock(&site, request), Ok(true));
    // The drive ended while the masterkey still existed; then it was wiped.
    assert_eq!(*participant.key_present.lock().unwrap(), vec![true]);
    assert!(session.is_locked());
    assert_eq!(consumers(&register, &fixture), 0);
    assert_eq!(last_lock_reason(&register, &site), Some(LockReason::Sleep));
}
