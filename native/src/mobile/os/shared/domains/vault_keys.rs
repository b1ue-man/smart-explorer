//! `vault.*` with keys and passwords (VLT-M11, U31-3 to U31-6, U31-12 and
//! the answers S1 and S4 of the security agent): detect, enter, create,
//! remove, unlock, change the password, the remembered password and the
//! vault sync job.
//!
//! S1: Android never stores a password in cleartext. „Passwort merken“
//! stores only the envelope `android-kw1:…` the app sealed with a Keystore
//! key that needs the device lock for every use; `vault.unlock` therefore
//! refuses `saved` and `remember`, and every call here uses
//! `NoPlaintextStore`, which refuses to store a password. S4: `vault.unlock`
//! is a task whose cancellation drops the derived key before the session
//! exists.
use serde_json::{json, Value};
use zeroize::Zeroizing;

use super::args::{invalid, opt_i64, opt_str, str_arg};
use super::vaults::{label_of, manage_error};
use crate::bisync::{DeletePolicy, Direction};
use crate::mobile::{ApiError, Runtime, TaskCtx};
use crate::vault::{
    CopyTarget, JobPlace, OsSecretStore, PasswordError, RegistryStore, SecretStore, SettingsChange,
    UnlockError, UnlockOptions, UnlockTarget,
};

/// The prefix of an envelope sealed by the app (S1).
const SEALED_PREFIX: &str = "android-kw1:";
/// S1: a remembered password needs the device lock on Android.
const SAVED_NEEDS_DEVICE_LOCK: &str =
    "Gespeicherte Passwörter brauchen auf Android die Bestätigung der Displaysperre.";
const NO_PLAINTEXT: &str = "Auf Android wird kein Passwort im Klartext gespeichert.";

pub(super) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "vault.detect" => detect(args),
        "vault.add" => add(args),
        "vault.remove" => remove(args),
        "vault.unlock" => unlock(rt, args),
        "vault.create" => create(rt, args),
        "vault.changePassword" => change_password(rt, args),
        "vault.rememberSaved" => remember_saved(args),
        "vault.savedSecret" => saved_secret(args),
        "vault.syncCreate" => sync_create(rt, args),
        _ => return None,
    })
}

/// The secret store of the app process without the write of a password:
/// reading and deleting reach the real store, storing is refused (S1).
pub(super) struct NoPlaintextStore;

impl SecretStore for NoPlaintextStore {
    fn set(&self, _account: &str, _secret: &str) -> Result<(), String> {
        Err(NO_PLAINTEXT.to_string())
    }

    fn get(&self, account: &str) -> Result<Option<String>, String> {
        OsSecretStore.get(account)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        OsSecretStore.delete(account)
    }
}

fn resolve(location: &str) -> Result<(crate::vfs::BackendHandle, String), String> {
    crate::connect::resolve_endpoint(location)
}

/// Memory scrypt may take: `memAvail − memThreshold` of the app (C1.3 (c)),
/// else what the platform reads.
pub(super) fn memory(args: &Value) -> Option<u64> {
    match (opt_i64(args, "memAvail"), opt_i64(args, "memThreshold")) {
        (Some(available), threshold) => {
            let free = available.saturating_sub(threshold.unwrap_or(0)).max(0);
            u64::try_from(free).ok()
        }
        _ => crate::vault::available_memory(),
    }
}

pub(super) fn unlock_error(error: &UnlockError) -> ApiError {
    let kind = error.code().unwrap_or(match error {
        UnlockError::Unknown(_) => "not_found",
        UnlockError::Unreachable(_) => "network",
        UnlockError::OtherVault => "conflict",
        _ => "invalid",
    });
    ApiError::new(kind, error.to_string())
}

fn password_error(error: &PasswordError) -> ApiError {
    match error {
        PasswordError::Unlock(unlock) => unlock_error(unlock),
        PasswordError::NewPassword(_) => ApiError::new("invalid", error.to_string()),
        PasswordError::Storage(_) => ApiError::new("network", error.to_string()),
        PasswordError::NotAtomic => {
            ApiError::new(error.code().unwrap_or("unsupported"), error.to_string())
        }
    }
}

/// Fails the task with `error` and keeps its code in the task result, so
/// the app can show the text of the code (U31-4, U31-11).
fn fail(ctx: &TaskCtx, error: ApiError) -> ApiError {
    ctx.set_failure_result(json!({ "code": error.kind, "message": error.message }));
    error
}

fn site_arg(args: &Value) -> Result<crate::vault::SiteRecord, ApiError> {
    RegistryStore::open_default()
        .resolve(str_arg(args, "site")?)
        .map_err(ApiError::not_found)
}

/// `vault.detect {location}` (T2, U31-3).
fn detect(args: &Value) -> Result<Value, ApiError> {
    let location = str_arg(args, "location")?;
    let found = crate::vault::detect_location(location, &resolve).map_err(manage_error)?;
    let name = location
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    Ok(json!({
        "isVault": found.is_vault,
        "format": found.format,
        "keySource": found.key_source,
        "vaultId": found.vault_id,
        "problem": found.problem,
        "name": name,
    }))
}

/// `vault.add {location, label?}` (T3): entered, still locked.
fn add(args: &Value) -> Result<Value, ApiError> {
    let record = crate::vault::add_site(
        &RegistryStore::open_default(),
        str_arg(args, "location")?,
        opt_str(args, "label"),
        &resolve,
    )
    .map_err(manage_error)?;
    Ok(json!({ "site": record.id, "label": record.display_label() }))
}

/// `vault.remove {site}` (T5): locks first; vault sync jobs keep it.
fn remove(args: &Value) -> Result<Value, ApiError> {
    let record = crate::vault::remove_entered(str_arg(args, "site")?).map_err(manage_error)?;
    Ok(json!({ "site": record.id }))
}

/// `vault.unlock {site, password, memAvail?, memThreshold?}` (task, T6).
fn unlock(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    if args.get("saved").and_then(Value::as_bool) == Some(true)
        || args.get("remember").and_then(Value::as_bool) == Some(true)
    {
        return Err(invalid(SAVED_NEEDS_DEVICE_LOCK));
    }
    let record = site_arg(args)?;
    let password = Zeroizing::new(str_arg(args, "password")?.to_string());
    let available = memory(args);
    let title = format!("Tresor {} entsperren", record.display_label());
    let id = rt.spawn_task("vault", title, move |ctx| {
        let cancel = ctx.cancel_flag();
        let store = RegistryStore::open_default();
        let target = UnlockTarget {
            register: crate::vault::register(),
            store: &store,
            secrets: &NoPlaintextStore,
            query: &record.id,
        };
        let options = UnlockOptions {
            remember: None,
            available,
        };
        crate::vault::unlock_entered_cancellable(target, &password, options, &resolve, &cancel)
            .map(|unlocked| json!({ "site": unlocked.site, "already": unlocked.already }))
            .map_err(|error| fail(ctx, unlock_error(&error)))
    });
    Ok(json!({ "taskId": id }))
}

/// `vault.create {location, label?, password}` (task, T4, U31-5): creates,
/// enters and unlocks with the password just given.
fn create(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let location = str_arg(args, "location")?.to_string();
    let label = opt_str(args, "label").map(str::to_string);
    let password = Zeroizing::new(str_arg(args, "password")?.to_string());
    crate::vault::check_new_password(&password).map_err(invalid)?;
    let available = memory(args);
    let title = match &label {
        Some(label) => format!("Tresor {label} anlegen"),
        None => "Tresor anlegen".to_string(),
    };
    let id = rt.spawn_task("vault", title, move |ctx| {
        let store = RegistryStore::open_default();
        let record = crate::vault::create_site(
            &store,
            &location,
            label.as_deref(),
            &password,
            crate::cryptomator::DEFAULT_SHORTENING_THRESHOLD,
            available,
            &resolve,
        )
        .map_err(|error| fail(ctx, manage_error(error)))?;
        let unlocked = crate::vault::unlock_entered(
            crate::vault::register(),
            &store,
            &NoPlaintextStore,
            &record.id,
            &password,
            UnlockOptions {
                remember: None,
                available,
            },
            &resolve,
        );
        Ok(json!({
            "site": record.id,
            "label": record.display_label(),
            "unlocked": unlocked.is_ok(),
            "unlockError": unlocked.err().map(|error| error.to_string()),
        }))
    });
    Ok(json!({ "taskId": id }))
}

/// `vault.changePassword {site, oldPassword, newPassword}` (task, T10). A
/// remembered envelope holds the old password, so it is deleted; the app
/// offers „Passwort merken“ again at the next unlock.
fn change_password(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let record = site_arg(args)?;
    let old = Zeroizing::new(str_arg(args, "oldPassword")?.to_string());
    let new = Zeroizing::new(str_arg(args, "newPassword")?.to_string());
    crate::vault::check_new_password(&new).map_err(invalid)?;
    let available = memory(args);
    let title = format!("Passwort von Tresor {} ändern", record.display_label());
    let id = rt.spawn_task("vault", title, move |ctx| {
        let store = RegistryStore::open_default();
        let changed = crate::vault::change_site_password(
            &store,
            &NoPlaintextStore,
            &record.id,
            &old,
            &new,
            available,
            &resolve,
        )
        .map_err(|error| fail(ctx, password_error(&error)))?;
        let cleared = record.remember_password
            && crate::vault::update_settings(
                &store,
                &record.id,
                &SettingsChange {
                    remember_password: Some(false),
                    ..SettingsChange::default()
                },
                &OsSecretStore,
            )
            .is_ok();
        Ok(json!({ "backup": changed.backup, "rememberCleared": cleared }))
    });
    Ok(json!({ "taskId": id }))
}

/// `vault.rememberSaved {site, sealed}` (S1): stores the envelope the app
/// sealed after the device lock confirmed it. Only for an unlocked site, so
/// the password inside has just opened the vault.
fn remember_saved(args: &Value) -> Result<Value, ApiError> {
    let record = site_arg(args)?;
    let sealed = str_arg(args, "sealed")?;
    if !is_envelope(sealed) {
        return Err(invalid(
            "„sealed“ ist kein Umschlag der App (android-kw1:…).",
        ));
    }
    if crate::vault::register().session(&record.id).is_none() {
        return Err(ApiError::from(crate::vault::vault_locked(&record.id)));
    }
    OsSecretStore
        .set(&crate::vault::account(&record.id), sealed)
        .map_err(|error| ApiError::internal(format!("Passwort merken: {error}")))?;
    let store = RegistryStore::open_default();
    let mut current = store
        .get(&record.id)
        .map_err(|error| ApiError::internal(format!("Tresorliste lesen: {error}")))?
        .ok_or_else(|| ApiError::not_found("Tresorort ist nicht eingetragen"))?;
    current.remember_password = true;
    store.save(&current).map_err(ApiError::internal)?;
    Ok(json!({}))
}

/// Whether `text` is an envelope of the app: the prefix and Base64url of at
/// least the IV (12 bytes) and the GCM tag (16 bytes).
pub(super) fn is_envelope(text: &str) -> bool {
    let body = text.strip_prefix(SEALED_PREFIX).unwrap_or_default();
    body.len() >= 38
        && body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// `vault.savedSecret {site}` → `{sealed}` (S1): the envelope; opening it
/// needs the app's Keystore key and the device lock.
fn saved_secret(args: &Value) -> Result<Value, ApiError> {
    let record = site_arg(args)?;
    let none = || {
        ApiError::new(
            "vault_no_saved_password",
            "Für diesen Tresor ist kein Passwort gemerkt",
        )
    };
    if !record.remember_password {
        return Err(none());
    }
    let sealed = OsSecretStore
        .get(&crate::vault::account(&record.id))
        .map_err(|error| ApiError::internal(format!("Gemerktes Passwort lesen: {error}")))?
        .ok_or_else(none)?;
    Ok(json!({ "sealed": sealed }))
}

/// `vault.syncCreate {site, target: "replica"|"site"|"location", with?,
/// name?, direction?, deletePolicy?}` (T15, U31-12): the vault sync job over
/// `vault::create_vault_job`; it moves only encrypted files.
fn sync_create(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let record = site_arg(args)?;
    let with = opt_str(args, "with").unwrap_or_default().trim().to_string();
    let target = match str_arg(args, "target")? {
        "replica" => CopyTarget::Replica,
        "site" if !with.is_empty() => CopyTarget::Site(&with),
        "location" if !with.is_empty() => CopyTarget::Location(&with),
        _ => {
            return Err(invalid(
                "„target“ ist replica, site oder location (mit „with“).",
            ))
        }
    };
    let name = opt_str(args, "name")
        .map(str::to_string)
        .unwrap_or_else(|| format!("Tresor {}", label_of(&record.id)));
    let direction = match opt_str(args, "direction") {
        None => Direction::Both,
        Some(key) => Direction::parse(key).ok_or_else(|| invalid("Unbekannte Richtung"))?,
    };
    let policy = match (direction, opt_str(args, "deletePolicy")) {
        (Direction::Both, _) | (_, None) => DeletePolicy::Propagate,
        (_, Some(key)) => {
            DeletePolicy::parse(key).ok_or_else(|| invalid("Unbekannte Löschregel"))?
        }
    };
    let replicas = crate::vault::default_replicas_root();
    let taken = |id: &str| crate::syncjobs::job_id_taken(id);
    let save = |job: &crate::syncjobs::SyncJob| -> Result<(), String> {
        let mut editor = crate::syncjobs::editor::JobEditor::from_job(job);
        editor.name = name.clone();
        editor.direction = direction;
        editor.delete_policy = policy;
        editor.move_files = false;
        let built = editor.build_sync_job(Some(job))?;
        crate::syncjobs::upsert(&built).map_err(|error| error.to_string())
    };
    let place = JobPlace {
        replicas_root: &replicas,
        resolve: &resolve,
        id_taken: &taken,
        save: &save,
    };
    let created = crate::vault::create_vault_job(
        &RegistryStore::open_default(),
        &record.id,
        target,
        Some(name.as_str()),
        &place,
    )
    .map_err(|error| ApiError::new("invalid", error))?;
    rt.emit(json!({ "type": "jobs" }));
    Ok(json!({
        "job": created.job.id,
        "name": name,
        "siteB": created.site_b.id,
        "createdSite": created.created_site,
    }))
}

#[cfg(test)]
#[path = "vault_keys_tests.rs"]
mod tests;
