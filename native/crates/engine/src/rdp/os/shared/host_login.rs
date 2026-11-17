//! Host side of the sign-in with the grant's account (plan 2026-11-17 P4,
//! TODO RDP-ZIELKONTO): which account (`AccountProfile` of the grant, as
//! Exec and the machine login), how the viewer signs in (`automatic`,
//! `stored`, `entry_needed`, `unsupported:<grund>`), the passwords viewers
//! typed once and at most five wrong ones per viewer and hour. A stored
//! password lies in the credential store of this worker's account (Windows
//! Credential Manager, Linux sealed store), one per viewer and account; an
//! index without secrets names them. „Bildschirm steuern“ ending for a viewer
//! deletes its passwords and tokens (`coverage_ended`). The platform adapter
//! names the account, checks a password and issues automatic secrets.
use super::super::platform::host_login as platform;
use crate::rdp::core::{valid_password, HostAccount, LoginInfo, LoginState};
use crate::share::access_grant::AccountProfile;
use crate::share::apps_wire::ManagedSecret;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// At most this many wrong passwords per viewer within `FAILURE_WINDOW`.
pub const MAX_FAILURES: usize = 5;
pub const FAILURE_WINDOW: Duration = Duration::from_secs(3600);

/// The viewer device of one request (proven relation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewer {
    pub device_id: String,
    pub public_key: String,
}
impl Viewer {
    pub fn new(device_id: &str, public_key: &str) -> Self {
        Self {
            device_id: device_id.to_string(),
            public_key: public_key.to_string(),
        }
    }
    /// The one-line binding of tokens and stored passwords.
    pub fn binding(&self) -> String {
        format!("{}|{}", self.device_id, self.public_key)
    }
}

/// What the platform does without a typed password.
pub enum Automatic {
    /// Nothing automatic for this account.
    No,
    /// Automatic, asked without side effect (`status_only`).
    Available,
    /// The secret of this session.
    Secret(ManagedSecret),
    /// Automatic would apply but cannot run; the reason.
    Unsupported(String),
}

/// Steering is checked before and after `work`; a right that ended meanwhile
/// undoes the work and refuses (as `RdpCredential`, threat T-R2).
pub fn with_steering<T>(
    check: impl Fn() -> Result<(), String>,
    work: impl FnOnce() -> Result<T, String>,
    undo: impl FnOnce(&T),
) -> Result<T, String> {
    check()?;
    let done = work()?;
    if let Err(error) = check() {
        undo(&done);
        return Err(error);
    }
    Ok(done)
}

/// `RdpLogin`: the account of the grant and how the viewer signs in.
/// `status_only` answers without a password and without side effect.
pub fn login_info(
    service_id: &str,
    viewer: &Viewer,
    profile: &AccountProfile,
    status_only: bool,
) -> Result<LoginInfo, String> {
    profile.validate()?;
    if let Err(reason) = platform::supports(service_id) {
        return Ok(LoginInfo::unsupported(reason));
    }
    let account = match platform::account(profile) {
        Ok(account) => account,
        Err(reason) => return Ok(LoginInfo::unsupported(reason)),
    };
    let automatic = platform::automatic(service_id, profile, &account, viewer, status_only);
    let info = |secret, state| LoginInfo::new(Some(account.clone()), secret, state);
    match automatic {
        Automatic::Secret(secret) => return Ok(info(Some(secret), LoginState::Automatic)),
        Automatic::Available => return Ok(info(None, LoginState::Automatic)),
        Automatic::Unsupported(reason) => return Ok(LoginInfo::unsupported(reason)),
        Automatic::No => {}
    }
    let Some(entry) = stored_entry(viewer, &account) else {
        return Ok(info(None, LoginState::EntryNeeded));
    };
    if status_only {
        return Ok(info(None, LoginState::Stored));
    }
    let secret = match crate::creds::get_secret_checked(&entry.credential()) {
        Ok(Some(secret)) => ManagedSecret::new(secret),
        Ok(None) => {
            forget_entries(|known| known == &entry);
            return Ok(info(None, LoginState::EntryNeeded));
        }
        Err(error) => return Err(format!("Hinterlegtes Kennwort nicht lesbar: {error}")),
    };
    // A password changed on this device meanwhile is forgotten; a check that
    // cannot run (logon type not granted, service down) keeps the stored one.
    if let Ok(false) = platform::verify(&account, secret.expose()) {
        forget_entries(|known| known == &entry);
        return Ok(info(None, LoginState::EntryNeeded));
    }
    Ok(info(Some(secret), LoginState::Stored))
}

/// `RdpStorePassword`: checks the password of the grant's account and
/// stores it for this viewer; `None` forgets it. Answers the new state.
pub fn store_password(
    service_id: &str,
    viewer: &Viewer,
    profile: &AccountProfile,
    password: Option<&ManagedSecret>,
) -> Result<LoginInfo, String> {
    profile.validate()?;
    platform::supports(service_id).map_err(|reason| {
        format!("Anmeldung mit dem Konto des Geräts hier nicht möglich ({reason})")
    })?;
    let account = platform::account(profile)?;
    let Some(password) = password else {
        let binding = viewer.binding();
        forget_entries(|known| known.binding() == binding && known.account == account.qualified());
        return login_info(service_id, viewer, profile, true);
    };
    // An account that signs in automatically (or would, but cannot here)
    // takes no typed password: its password is se's own.
    match platform::automatic(service_id, profile, &account, viewer, true) {
        Automatic::Available | Automatic::Secret(_) => {
            return Err(format!(
                "{} meldet sich automatisch an; es braucht kein Kennwort",
                account.qualified()
            ))
        }
        Automatic::Unsupported(reason) => return Err(reason),
        Automatic::No => {}
    }
    if !valid_password(password.expose()) {
        return Err("Das Kennwort ist leer, zu lang oder enthält einen Zeilenumbruch".into());
    }
    let key = viewer.binding();
    let left = failures_left(&key, Instant::now());
    if left == 0 {
        return Err(format!(
            "Zu viele falsche Kennwörter von diesem Gerät; erneut in spätestens einer Stunde (höchstens {MAX_FAILURES} je Stunde)"
        ));
    }
    if !platform::verify(&account, password.expose())? {
        note_failure(&key, Instant::now());
        return Err(format!(
            "Das Kennwort von {} stimmt nicht; noch {} Versuche in dieser Stunde",
            account.qualified(),
            left - 1
        ));
    }
    let entry = StoredEntry {
        device_id: viewer.device_id.clone(),
        public_key: viewer.public_key.clone(),
        account: account.qualified(),
        at: crate::share::core_now_secs(),
    };
    // Not `set_secret`: no sync job waits for this credential.
    crate::creds::set_secret_untracked(&entry.credential(), password.expose())?;
    let _index = INDEX.lock().unwrap_or_else(PoisonError::into_inner);
    let mut entries = read_index();
    entries.retain(|known| known.credential() != entry.credential());
    entries.push(entry);
    write_index(&entries)?;
    Ok(LoginInfo::new(Some(account), None, LoginState::Stored))
}

/// Undoes what a request issued when its right ended meanwhile: the
/// viewer's tokens, a se-remote password and its stored passwords.
pub fn withdraw(viewer: &Viewer, issued: &LoginInfo) {
    platform::withdraw(viewer, issued);
    let binding = viewer.binding();
    forget_entries(|known| known.binding() == binding);
}

/// „Bildschirm steuern“ ended for some viewer (`self_setup::coverage_ended`):
/// stored passwords of viewers that no longer steer are deleted, their
/// tokens revoked, and `se-remote` gets a password nobody receives.
pub fn coverage_ended(steering_remains: bool) {
    let steers = |viewer: &Viewer| {
        crate::share::access_steering::steering_device(&viewer.device_id, &viewer.public_key)
    };
    forget_entries(|known| !steers(&Viewer::new(&known.device_id, &known.public_key)));
    platform::coverage_ended(&steers, steering_remains);
}

/// How many passwords of this device's accounts other devices stored here
/// („Dieses Gerät“).
pub fn stored_count() -> usize {
    read_index().len()
}

/// Deletes every password other devices stored here („Dieses Gerät“).
pub fn forget_all() {
    forget_entries(|_| true);
}

/// One stored password, without the secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEntry {
    pub device_id: String,
    pub public_key: String,
    /// `DOMAIN\user` of the account.
    pub account: String,
    pub at: i64,
}
impl StoredEntry {
    fn binding(&self) -> String {
        Viewer::new(&self.device_id, &self.public_key).binding()
    }
    /// The credential-store name: a digest, so no device data appears in it.
    pub fn credential(&self) -> String {
        let digest = Sha256::digest(format!("{}|{}", self.binding(), self.account).as_bytes());
        format!("smart-explorer:rdp-login:{digest:x}")
    }
}

/// The entries `steers` keeps and those it drops (pure part of
/// `coverage_ended`).
pub fn forget_unsteered(
    entries: Vec<StoredEntry>,
    steers: impl Fn(&StoredEntry) -> bool,
) -> (Vec<StoredEntry>, Vec<StoredEntry>) {
    entries.into_iter().partition(|entry| steers(entry))
}

static INDEX: Mutex<()> = Mutex::new(());

fn index_path() -> PathBuf {
    crate::support_dirs::sync_data_dir().join("rdp-login-stored.json")
}

fn read_index() -> Vec<StoredEntry> {
    std::fs::read_to_string(index_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_index(entries: &[StoredEntry]) -> Result<(), String> {
    let path = index_path();
    let bytes = serde_json::to_vec(entries).map_err(|error| error.to_string())?;
    let staged = path.with_extension("json.tmp");
    std::fs::write(&staged, bytes)
        .and_then(|()| std::fs::rename(&staged, &path))
        .map_err(|error| {
            format!("Verzeichnis hinterlegter RDP-Kennwörter nicht speicherbar: {error}")
        })
}

/// Deletes the stored passwords `remove` names and their index entries; an
/// entry whose secret could not be deleted stays listed for the next try.
fn forget_entries(remove: impl Fn(&StoredEntry) -> bool) {
    let _index = INDEX.lock().unwrap_or_else(PoisonError::into_inner);
    let (kept, gone) = forget_unsteered(read_index(), |entry| !remove(entry));
    if gone.is_empty() {
        return;
    }
    let mut entries = kept;
    for entry in gone {
        if let Err(error) = crate::creds::delete_secret_checked(&entry.credential()) {
            eprintln!("RDP-Anmeldung: hinterlegtes Kennwort nicht gelöscht: {error}");
            entries.push(entry);
        }
    }
    if let Err(error) = write_index(&entries) {
        eprintln!("RDP-Anmeldung: {error}");
    }
}

fn stored_entry(viewer: &Viewer, account: &HostAccount) -> Option<StoredEntry> {
    let (binding, qualified) = (viewer.binding(), account.qualified());
    read_index()
        .into_iter()
        .find(|entry| entry.binding() == binding && entry.account == qualified)
}

/// Wrong passwords per viewer binding, in memory (a worker restart resets).
static FAILURES: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());

fn failures_left(key: &str, now: Instant) -> usize {
    let mut failures = FAILURES.lock().unwrap_or_else(PoisonError::into_inner);
    failures.retain(|(_, at)| now.duration_since(*at) < FAILURE_WINDOW);
    let used = failures.iter().filter(|(known, _)| known == key).count();
    MAX_FAILURES.saturating_sub(used)
}

fn note_failure(key: &str, now: Instant) {
    FAILURES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((key.to_string(), now));
}

#[cfg(test)]
#[path = "host_login_task_tests.rs"]
mod host_login_task_tests;
