//! U31 S4: „Abbrechen“ while the key is derived leaves the vault locked.
use std::sync::atomic::AtomicBool;

use crate::vault::core::lock_policy::LockPlatform;

use super::super::session::{RegisterOptions, SessionRegister};
use super::super::session_test_support::{local_resolve, site_fixture, MemorySecrets};
use super::{unlock_entered_cancellable, UnlockError, UnlockOptions, UnlockTarget};

const PASSWORD: &str = "Tresor-Passwort-1";

#[test]
fn vault_task_android_unlock_cancel_discards_key() {
    let fixture = site_fixture("Abbruch", PASSWORD);
    let site = fixture.record.id.clone();
    let register = SessionRegister::new(RegisterOptions {
        publish: false,
        platform: LockPlatform::Android,
        background: false,
    });
    let secrets = MemorySecrets::default();
    let target = || UnlockTarget {
        register: &register,
        store: &fixture.store,
        secrets: &secrets,
        query: &site,
    };
    let remember = UnlockOptions {
        remember: Some(true),
        available: None,
    };
    let withdrawn = AtomicBool::new(true);
    let result =
        unlock_entered_cancellable(target(), PASSWORD, remember, &local_resolve, &withdrawn);
    assert_eq!(result.unwrap_err(), UnlockError::Cancelled);
    assert_eq!(UnlockError::Cancelled.code(), Some("canceled"));
    // No session, nothing remembered.
    assert!(register.session(&site).is_none());
    assert!(secrets.is_empty());
    // Without the flag the same call unlocks.
    let open = AtomicBool::new(false);
    let unlocked = unlock_entered_cancellable(
        target(),
        PASSWORD,
        UnlockOptions::default(),
        &local_resolve,
        &open,
    )
    .unwrap();
    assert!(!unlocked.already);
    assert!(register.session(&site).is_some());
}
