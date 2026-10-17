//! Sessions without storage, vaults on disk and a secret store in memory
//! for the tests of the session register, the management and the platform
//! adapters (stages `vault_unit`, `vault_linux`, `vault_windows`).
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::cryptomator::{CipherCombo, Masterkey, VaultConfig};
use crate::vault::core::lock_policy::LockPlatform;
use crate::vault::core::registry_types::{new_site_id, LockSettings, SiteRecord};
use crate::vault::core::VaultSession;
use crate::vfs::{BackendHandle, LocalBackend};

use super::backend_test_support::fwd;
use super::manage::create_site;
use super::registry_store::RegistryStore;
use super::remember::SecretStore;
use super::session::{RegisterOptions, SessionRegister};

/// The secret store of the tests.
#[derive(Default)]
pub(super) struct MemorySecrets {
    entries: Mutex<HashMap<String, String>>,
    fail_set: AtomicBool,
}

impl MemorySecrets {
    pub(super) fn entry(&self, account: &str) -> Option<String> {
        self.lock().get(account).cloned()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Makes every `set` fail (a store that refuses).
    pub(super) fn refuse_set(&self, refuse: bool) {
        self.fail_set.store(refuse, Ordering::SeqCst);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SecretStore for MemorySecrets {
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        if self.fail_set.load(Ordering::SeqCst) {
            return Err("Anmeldeinformationsspeicher gesperrt".into());
        }
        self.lock().insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self.entry(account))
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.lock().remove(account);
        Ok(())
    }
}

/// Every location is a local path (the vaults of the tests).
pub(super) fn local_resolve(location: &str) -> Result<(BackendHandle, String), String> {
    Ok((Arc::new(LocalBackend::new("/")), location.to_string()))
}

/// A vault created by se in a temporary folder, entered in a vault list
/// of its own.
pub(super) struct SiteFixture {
    pub(super) dir: tempfile::TempDir,
    pub(super) store: RegistryStore,
    pub(super) record: SiteRecord,
    /// The vault root (forward slashes).
    pub(super) root: String,
}

pub(super) fn site_fixture(label: &str, password: &str) -> SiteFixture {
    let dir = tempfile::tempdir().expect("temporary folder");
    let store = RegistryStore::at(dir.path().join("vaults"), LockSettings::desktop());
    let root = format!("{}/{label}", fwd(dir.path()));
    let record = create_site(
        &store,
        &root,
        Some(label),
        password,
        220,
        None,
        &local_resolve,
    )
    .expect("vault created and entered");
    SiteFixture {
        dir,
        store,
        record,
        root,
    }
}

/// A register of its own: no process table, no timer, no platform signals.
pub(super) fn test_register(platform: LockPlatform) -> Arc<SessionRegister> {
    SessionRegister::new(RegisterOptions {
        publish: false,
        platform,
        background: false,
    })
}

pub(super) fn settings(
    idle: Option<u32>,
    sleep: bool,
    screen: bool,
    background: Option<u32>,
) -> LockSettings {
    LockSettings {
        auto_lock_idle_min: idle,
        lock_on_sleep: sleep,
        lock_on_screen_lock: screen,
        lock_on_background_min: background,
    }
}

/// A session with a random key (no storage behind it).
pub(super) fn session(site: &str) -> Arc<VaultSession> {
    let key = Masterkey::generate().expect("random masterkey");
    let config = VaultConfig {
        jti: format!("jti-{site}"),
        combo: CipherCombo::SivGcm,
        shortening_threshold: 220,
    };
    VaultSession::new(site, key, config)
}

pub(super) fn record(site: &str, settings: LockSettings) -> SiteRecord {
    let label = format!("Tresor {}", &site[..6]);
    let location = format!("/tresore/{site}");
    SiteRecord::new(site, &label, &location, &format!("jti-{site}"), 1, settings)
}

/// A new site unlocked in `register`: its ID and its session.
pub(super) fn unlocked_site(
    register: &SessionRegister,
    settings: LockSettings,
) -> (String, Arc<VaultSession>) {
    let site = new_site_id().expect("site ID");
    let session = session(&site);
    let installed = register.install(&record(&site, settings), Arc::clone(&session));
    assert!(!installed.already);
    (site, session)
}

/// `site`, locked before, unlocked once more with a fresh session.
pub(super) fn unlock_again(
    register: &SessionRegister,
    site: &str,
    settings: LockSettings,
) -> Arc<VaultSession> {
    let session = session(site);
    let installed = register.install(&record(site, settings), Arc::clone(&session));
    assert!(!installed.already, "the site was locked before");
    session
}

pub(super) fn is_unlocked(register: &SessionRegister, site: &str) -> bool {
    register.session(site).is_some()
}

/// Waits up to 10 s for `check`: signals arrive on other threads.
pub(super) fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < until, "nicht eingetreten: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
