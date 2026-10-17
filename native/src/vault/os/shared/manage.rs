//! Managing vault sites (VLT-M5, C7.1 T2–T5, T12–T14): detect a vault at a
//! location, enter an existing one, create a new one, remove a site from the
//! vault list (the vault itself stays where it is) and change the lock
//! settings. Recovery keys of Cryptomator are not offered (C1.10, OE1): the
//! surfaces show `RECOVERY_KEY_NOTE`, there is no word list in se.
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::cryptomator::{FormatError, KeySource};
use crate::vault::core::lock_policy::{self, LockReason};
use crate::vault::core::registry_types::{location_key, new_site_id, SiteRecord};
use crate::vfs::Backend;

use super::open::ResolveLocation;
use super::registry_store::RegistryStore;
use super::remember::{self, OsSecretStore, SecretStore};
use super::session::{hosts_register, register, LockRequest};
use super::unlock::{read_key_files, require_config, UnlockError};

#[path = "manage_create.rs"]
mod create;

pub(crate) use create::{create_site, create_vault};

/// What every surface shows instead of recovery keys (C1.10, T12, T13).
pub(crate) const RECOVERY_KEY_NOTE: &str = "Wiederherstellungsschlüssel von Cryptomator werden \
in se nicht angezeigt und nicht angenommen, weil die Wortliste nicht frei lizenziert ist. Zum \
Anzeigen oder Zurücksetzen Cryptomator Desktop verwenden; der Tresor bleibt dafür voll kompatibel.";
/// The note of the creation wizard (C7.6).
pub(crate) const CREATE_RECOVERY_HINT: &str =
    "Einen Wiederherstellungsschlüssel erzeugt Cryptomator Desktop für diesen Tresor";
/// Read limit of one sync job file while looking for vault sites.
const MAX_JOB_FILE: u64 = 1024 * 1024;

/// Recovery keys in se (`se vault show`, `vault.show`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryKeyInfo {
    pub(crate) supported: bool,
    pub(crate) note: &'static str,
}

pub(crate) fn recovery_key_info() -> RecoveryKeyInfo {
    RecoveryKeyInfo {
        supported: false,
        note: RECOVERY_KEY_NOTE,
    }
}

/// What lies at a location (`se vault detect`, `vault.detect`, T2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Detection {
    pub(crate) is_vault: bool,
    pub(crate) format: Option<i64>,
    /// `masterkeyfile`, `hub` or `other`.
    pub(crate) key_source: Option<&'static str>,
    pub(crate) vault_id: Option<String>,
    /// Why se cannot open it (Hub, format, foreign key source).
    pub(crate) problem: Option<String>,
}

/// Why a management step did nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ManageError {
    /// Wrong input (password, label, settings, site name).
    Invalid(String),
    /// Not a vault se can open, or a vault already there.
    Refused(String),
    /// The storage location failed.
    Storage(String),
    /// The vault list failed.
    Registry(String),
    /// Vault sync jobs use the site (their IDs).
    InUse(Vec<String>),
}

impl fmt::Display for ManageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message)
            | Self::Refused(message)
            | Self::Storage(message)
            | Self::Registry(message) => f.write_str(message),
            Self::InUse(jobs) => write!(
                f,
                "Tresor-Sync-Jobs nutzen diesen Tresorort ({}): zuerst die Jobs entfernen",
                jobs.join(", ")
            ),
        }
    }
}

impl std::error::Error for ManageError {}

fn storage(error: impl fmt::Display) -> ManageError {
    ManageError::Storage(format!("Speicherort: {error}"))
}

fn refused(error: FormatError) -> ManageError {
    ManageError::Refused(error.to_string())
}

/// Detects a vault in the folder `root` on `inner` without a password.
pub(crate) fn detect_storage(inner: &dyn Backend, root: &str) -> Result<Detection, ManageError> {
    let files = match read_key_files(inner, root) {
        Ok(files) => files,
        Err(UnlockError::Unreachable(cause)) => return Err(storage(cause)),
        Err(error) => {
            return Ok(Detection {
                problem: Some(error.to_string()),
                ..Detection::default()
            })
        }
    };
    let config = match require_config(&files) {
        Ok(config) => config,
        Err(UnlockError::Format(FormatError::UnsupportedFormat(format))) => {
            return Ok(Detection {
                is_vault: true,
                format: Some(format),
                key_source: Some("masterkeyfile"),
                problem: Some(FormatError::UnsupportedFormat(format).to_string()),
                ..Detection::default()
            });
        }
        Err(error) => {
            return Ok(Detection {
                problem: Some(error.to_string()),
                ..Detection::default()
            })
        }
    };
    let format = config.alleged_format();
    let (key_source, mut problem) = match config.key_source() {
        Ok(KeySource::MasterkeyFile) => ("masterkeyfile", None),
        Err(FormatError::HubVault) => ("hub", Some(FormatError::HubVault.to_string())),
        Err(error) => ("other", Some(error.to_string())),
    };
    if problem.is_none() {
        problem = match format {
            Some(8) => None,
            Some(other) => Some(FormatError::UnsupportedFormat(other).to_string()),
            None => Some(FormatError::InvalidConfig("format fehlt".into()).to_string()),
        };
    }
    Ok(Detection {
        is_vault: true,
        format,
        key_source: Some(key_source),
        vault_id: config.alleged_vault_id(),
        problem,
    })
}

/// `detect_storage` of an se location.
pub(crate) fn detect_location(
    location: &str,
    resolve: ResolveLocation<'_>,
) -> Result<Detection, ManageError> {
    let (inner, root) = resolve(location.trim()).map_err(storage)?;
    detect_storage(inner.as_ref(), &root)
}

/// The last name of a location as the default label.
fn default_label(location: &str) -> String {
    location
        .trim()
        .split(['/', '\\'])
        .rev()
        .find(|part| !part.is_empty() && !part.ends_with(':'))
        .unwrap_or("Tresor")
        .to_string()
}

/// A label for a new site: the given one (unique among the sites), else the
/// last name of the location, numbered while taken.
fn new_label(
    store: &RegistryStore,
    wanted: Option<&str>,
    location: &str,
) -> Result<String, ManageError> {
    let load = store
        .load()
        .map_err(|error| ManageError::Registry(format!("Tresorliste lesen: {error}")))?;
    let taken = |label: &str| load.sites.iter().any(|site| site.label == label);
    let key = location_key(location);
    if let Some(site) = load
        .sites
        .iter()
        .find(|site| location_key(&site.location) == key)
    {
        return Err(ManageError::Refused(format!(
            "Dieser Speicherort ist schon als Tresorort „{}“ eingetragen",
            site.display_label()
        )));
    }
    if let Some(label) = wanted.map(str::trim).filter(|label| !label.is_empty()) {
        if taken(label) {
            return Err(ManageError::Invalid(format!(
                "Die Bezeichnung „{label}“ ist schon vergeben"
            )));
        }
        return Ok(label.to_string());
    }
    let base = default_label(location);
    let mut label = base.clone();
    let mut number = 2;
    while taken(&label) {
        label = format!("{base} ({number})");
        number += 1;
    }
    Ok(label)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn enter(
    store: &RegistryStore,
    location: &str,
    label: String,
    vault_id: &str,
) -> Result<SiteRecord, ManageError> {
    let id = new_site_id().map_err(ManageError::Registry)?;
    let record = SiteRecord::new(&id, &label, location, vault_id, now_ms(), store.defaults());
    store.add(&record).map_err(ManageError::Registry)?;
    Ok(record)
}

/// Enters the vault at `location` into the vault list (T3).
pub(crate) fn add_site(
    store: &RegistryStore,
    location: &str,
    label: Option<&str>,
    resolve: ResolveLocation<'_>,
) -> Result<SiteRecord, ManageError> {
    let location = location.trim();
    let label = new_label(store, label, location)?;
    let detection = detect_location(location, resolve)?;
    if let Some(problem) = detection.problem {
        return Err(ManageError::Refused(problem));
    }
    let vault_id = detection.vault_id.unwrap_or_default();
    enter(store, location, label, &vault_id)
}

/// The IDs of sync jobs whose file names the site (`vault_site_a`,
/// `vault_site_b`, `vault-store://<id>`, `vault://<id>`), in `dirs`.
pub(crate) fn jobs_using(site: &str, dirs: &[PathBuf]) -> Result<Vec<String>, String> {
    let needles = [format!("vault-store://{site}"), format!("vault://{site}")];
    let mut jobs = Vec::new();
    for dir in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Sync-Jobs lesen: {error}")),
        };
        for entry in entries {
            let path = entry
                .map_err(|error| format!("Sync-Jobs lesen: {error}"))?
                .path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("conf") {
                continue;
            }
            if job_uses(&path, site, &needles)? {
                if let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) {
                    jobs.push(id.to_string());
                }
            }
        }
    }
    jobs.sort();
    jobs.dedup();
    Ok(jobs)
}

fn job_uses(path: &Path, site: &str, needles: &[String]) -> Result<bool, String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_JOB_FILE).read_to_string(&mut text))
        .map_err(|error| format!("Sync-Job lesen: {error}"))?;
    Ok(text.lines().any(|line| {
        let (key, value) = line
            .split_once('=')
            .map_or(("", ""), |(key, value)| (key.trim(), value.trim()));
        let named = matches!(key, "vault_site_a" | "vault_site_b") && value == site;
        let lower = line.to_ascii_lowercase();
        named || needles.iter().any(|needle| lower.contains(needle.as_str()))
    }))
}

/// Removes the site `query` from the vault list (T5): refused while vault
/// sync jobs use it; locks it first (`lock`), forgets a remembered password,
/// drops se's sealed data of places inside it. The vault stays untouched.
pub(crate) fn remove_site(
    store: &RegistryStore,
    query: &str,
    job_dirs: &[PathBuf],
    secrets: &dyn SecretStore,
    lock: &dyn Fn(&str) -> Result<(), String>,
) -> Result<SiteRecord, ManageError> {
    let record = store.resolve(query).map_err(ManageError::Invalid)?;
    let jobs = jobs_using(&record.id, job_dirs).map_err(ManageError::Registry)?;
    if !jobs.is_empty() {
        return Err(ManageError::InUse(jobs));
    }
    lock(&record.id).map_err(ManageError::Refused)?;
    remember::forget(secrets, store, &record).map_err(ManageError::Registry)?;
    // se's own sealed data about places in the vault (K13, `<id>.local`).
    let _ = std::fs::remove_file(store.dir().join(format!("{}.local", record.id)));
    store.remove(&record.id).map_err(ManageError::Registry)?;
    Ok(record)
}

/// `remove_site` with the vault list, sync jobs and secret store of this
/// installation; locks through the register here or through the worker.
pub(crate) fn remove_entered(query: &str) -> Result<SiteRecord, ManageError> {
    let sync = crate::support_dirs::sync_data_dir();
    let dirs = [sync.join("jobs"), sync.join("vault-jobs")];
    let lock = |site: &str| -> Result<(), String> {
        if hosts_register() {
            register()
                .lock(site, LockRequest::new(LockReason::Removed, false))
                .map(|_| ())
                .map_err(|error| error.to_string())
        } else {
            crate::daemon::lock_vaults(Some(site), false)
                .map(|_| ())
                .map_err(|error| error.message)
        }
    };
    remove_site(
        &RegistryStore::open_default(),
        query,
        &dirs,
        &OsSecretStore,
        &lock,
    )
}

/// A change of the settings of a site (`se vault settings`, T14); `None`
/// keeps a value, `Some(None)` switches a timed lock off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SettingsChange {
    pub(crate) label: Option<String>,
    pub(crate) auto_lock_idle_min: Option<Option<u32>>,
    pub(crate) lock_on_sleep: Option<bool>,
    pub(crate) lock_on_screen_lock: Option<bool>,
    pub(crate) lock_on_background_min: Option<Option<u32>>,
    /// `Some(false)` forgets the password; it is remembered by an unlock.
    pub(crate) remember_password: Option<bool>,
}

/// Applies `change` to the site `query` and saves it; the register of this
/// process takes the new lock settings at once, the worker's at its next
/// look (status request or timer).
pub(crate) fn update_settings(
    store: &RegistryStore,
    query: &str,
    change: &SettingsChange,
    secrets: &dyn SecretStore,
) -> Result<SiteRecord, ManageError> {
    let mut record = store.resolve(query).map_err(ManageError::Invalid)?;
    if let Some(label) = &change.label {
        let label = label.trim();
        if label.is_empty() {
            return Err(ManageError::Invalid(
                "Bezeichnung darf nicht leer sein".into(),
            ));
        }
        record.label = label.to_string();
    }
    let lock = &mut record.lock;
    if let Some(idle) = change.auto_lock_idle_min {
        lock.auto_lock_idle_min = idle;
    }
    if let Some(sleep) = change.lock_on_sleep {
        lock.lock_on_sleep = sleep;
    }
    if let Some(screen) = change.lock_on_screen_lock {
        lock.lock_on_screen_lock = screen;
    }
    if let Some(background) = change.lock_on_background_min {
        lock.lock_on_background_min = background;
    }
    lock_policy::check_settings(lock).map_err(ManageError::Invalid)?;
    match change.remember_password {
        Some(true) if !record.remember_password => return Err(ManageError::Invalid(
            "Passwort merken wird beim Entsperren eingeschaltet (se vault unlock --remember true)"
                .into(),
        )),
        Some(false) => {
            remember::forget(secrets, store, &record).map_err(ManageError::Registry)?;
            record.remember_password = false;
        }
        _ => {}
    }
    store.save(&record).map_err(ManageError::Registry)?;
    if hosts_register() {
        register().reload_settings(std::slice::from_ref(&record));
    }
    Ok(record)
}
