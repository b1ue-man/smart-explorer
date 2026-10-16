//! `init` (api.md §1): host values, temp root, app trash volumes, browser
//! opener, crash log, runtime, then background housekeeping and the nudge of
//! the embedded daemon. Nothing here waits on the network or the daemon.
//! Scheduled daemon jobs wait for the host's first `sys.hostState`.
use super::config::HostSettings;
use super::error::ApiError;
use super::runtime::{lock, Runtime};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Cache subfolders declared in `file_paths.xml`.
const CACHE_SUBDIRS: [&str; 4] = ["open", "share", "update", "tmp"];
/// Shared copies older than this are removed at start.
const SHARE_COPY_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const TRASH_RETENTION_DAYS: u32 = 30;

pub(crate) fn init(config: &str) -> Result<Value, ApiError> {
    static INIT: Mutex<()> = Mutex::new(());
    let _serialized = lock(&INIT);
    let settings = HostSettings::parse(config)?;
    if let Some(runtime) = Runtime::installed() {
        runtime.set_volumes(settings.volumes);
        runtime.emit(json!({ "type": "volumes" }));
        return Ok(info(runtime));
    }
    let key_warning = install_secrets_key(config);
    prepare_host(&settings)?;
    // The data directory is known now. Seal plaintext records of earlier
    // versions before anything reads a secret and before the embedded worker
    // starts Share; a failure is only logged below and retried at the next
    // access.
    let port_result = crate::creds::port_secret_store();
    crate::install_panic_logger();
    // A process started in the background must not enqueue scheduled jobs
    // before the host said whether its own worker owns them.
    crate::daemon::defer_scheduling_until_reported();
    let runtime = Runtime::install(Runtime::detached(settings));
    runtime.set_volumes(runtime.config().volumes.clone());
    if let Some(warning) = key_warning {
        runtime.log_error("Geheimnisspeicher-Schlüssel", &warning);
    }
    if let Err(error) = port_result {
        runtime.log_error("Geheimnisspeicher portieren", &error);
    }
    crate::cloud::set_url_opener(Box::new(move |url: &str| {
        runtime.emit(json!({ "type": "openUrl", "url": url }));
    }));
    super::domains::install_activity_hook();
    if runtime.config().start_daemon {
        nudge_daemon(runtime);
    }
    start_housekeeping(runtime);
    super::domains::on_init(runtime);
    Ok(info(runtime))
}

fn info(runtime: &Runtime) -> Value {
    json!({
        "coreVersion": env!("CARGO_PKG_VERSION"),
        "dataDir": runtime.config().data_dir().to_string_lossy(),
    })
}

/// `secretsKey` (api.md §1): the credential store's data key, which the app
/// keeps wrapped by a non-exportable Android Keystore key (`SecretsKey.kt`).
/// Installed before anything reads a credential; it is not kept in the
/// settings. A missing key (Keystore failure) leaves sealed records closed
/// and the store reports why (`creds::key_protector`). A malformed key never
/// stops the start either: the returned warning (never the key) is logged and
/// the sealed records stay closed until a valid key arrives.
fn install_secrets_key(config: &str) -> Option<String> {
    let value: Value = serde_json::from_str(config).ok()?;
    let hex = value.get("secretsKey").and_then(Value::as_str)?;
    let Some(bytes) = crate::creds::sealed_record::hex_decode(hex).map(zeroize::Zeroizing::new)
    else {
        return Some("secretsKey ist kein Hex; versiegelte Einträge bleiben geschlossen".into());
    };
    crate::creds::install_host_data_key(&bytes).err()
}

fn prepare_host(settings: &HostSettings) -> Result<(), ApiError> {
    let device_name = if settings.device_name.trim().is_empty() {
        "Android-Gerät".to_string()
    } else {
        settings.device_name.trim().to_string()
    };
    crate::support_dirs::set_host(crate::support_dirs::HostConfig {
        data_home: settings.files_dir.clone(),
        cache_dir: settings.cache_dir.clone(),
        home_dir: settings.home(),
        device_name,
        boot_marker: settings.boot_marker.clone(),
    });
    for name in CACHE_SUBDIRS {
        std::fs::create_dir_all(settings.cache_subdir(name))
            .map_err(|error| ApiError::from(error).context("Cache-Ordner anlegen"))?;
    }
    std::fs::create_dir_all(settings.mobile_dir())
        .map_err(|error| ApiError::from(error).context("Datenordner anlegen"))?;
    if settings.home_dir.is_none() && settings.volumes.is_empty() {
        // The empty fallback home of `HostSettings::home` (no storage reported).
        let _ = std::fs::create_dir_all(settings.home());
    }
    let temp = crate::support_dirs::temp_dir();
    if let Err(existing) = tempfile::env::override_temp_dir(&temp) {
        if existing != temp {
            return Err(ApiError::internal(format!(
                "Temp-Ordner ist bereits auf {} gesetzt",
                existing.display()
            )));
        }
    }
    Ok(())
}

/// Starts the embedded daemon thread without waiting for it.
fn nudge_daemon(runtime: &'static Runtime) {
    let spawned = std::thread::Builder::new()
        .name("mobile-daemon-nudge".into())
        .spawn(move || {
            if let Err(error) = crate::daemon::ensure_embedded_daemon(Duration::ZERO) {
                runtime.log_error("Hintergrund-Worker starten", &error);
            }
        });
    if let Err(error) = spawned {
        runtime.log_error("Hintergrund-Worker starten", &error.to_string());
    }
}

/// Cleanup of leftovers from earlier process runs, off the calling thread.
fn start_housekeeping(runtime: &'static Runtime) {
    let spawned = std::thread::Builder::new()
        .name("mobile-housekeeping".into())
        .spawn(move || {
            sweep_transfer_temp(runtime);
            remove_old_children(
                runtime,
                &runtime.config().cache_subdir("share"),
                SHARE_COPY_MAX_AGE,
            );
            match crate::apptrash::purge_older_than(TRASH_RETENTION_DAYS) {
                Ok(_) => {}
                Err(error) => runtime.record_error("Papierkorb aufräumen", &error.to_string()),
            }
            super::edits::check_on_start(runtime);
        });
    if let Err(error) = spawned {
        runtime.record_error("Aufräumen", &error.to_string());
    }
}

/// Android ends the process without a signal, so transfer temp sessions of
/// earlier runs are removed here instead of at exit.
fn sweep_transfer_temp(runtime: &Runtime) {
    let root = crate::transfer::temp_root();
    let current = crate::transfer::session_temp_dir();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if !is_dir || path == current {
            continue;
        }
        if let Err(error) = crate::transfer::remove_owned_tree(&root, &path) {
            runtime.record_error(
                "Temporäre Dateien aufräumen",
                &format!("{}: {error}", path.display()),
            );
        }
    }
}

/// Removes direct children of `dir` not modified within `max_age`.
fn remove_old_children(runtime: &Runtime, dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if !old {
            continue;
        }
        if let Err(error) = crate::transfer::remove_owned_tree(dir, &path) {
            runtime.record_error(
                "Geteilte Kopien aufräumen",
                &format!("{}: {error}", path.display()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::install_secrets_key;

    #[test]
    fn luecken_task_a_malformed_secrets_key_is_a_warning_not_a_failed_start() {
        let warning = install_secrets_key(r#"{"secretsKey":"not-hex-at-all"}"#)
            .expect("a malformed key must be reported");
        assert!(!warning.contains("not-hex-at-all"));
        let warning = install_secrets_key(r#"{"secretsKey":"abcd"}"#)
            .expect("a key of the wrong length must be reported");
        assert!(!warning.contains("abcd"));
    }

    #[test]
    fn luecken_task_a_missing_secrets_key_is_not_a_warning() {
        assert_eq!(install_secrets_key(r#"{"secretsKey":null}"#), None);
        assert_eq!(install_secrets_key(r#"{"filesDir":"/x"}"#), None);
    }
}
