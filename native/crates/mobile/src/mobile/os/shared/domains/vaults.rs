//! `vault.*` of the Android facade (VLT-M11, C7.5, festlegungen § U31 with
//! the answers S1–S8 of the security agent): the vault list, lock settings,
//! locking and the `vault` event. The key work (unlock, create, password,
//! remembered password, entering, sync jobs) is in `vault_keys.rs`, the
//! comparison and the check in `vault_compare.rs`.
//!
//! The session register lives in this process (worker in process), so every
//! call reaches `crate::vault::register()` directly. Replies, events, task
//! titles and error texts carry site IDs, labels and states only; a
//! cleartext path from inside a vault appears only in the entries of a
//! comparison, which the app shows on screens with `FLAG_SECURE` (S2, S6).
use serde_json::{json, Map, Value};

use super::args::{invalid, opt_bool, opt_str, str_arg};
use crate::mobile::{ApiError, Runtime};
use crate::vault::{
    LockError, LockReason, LockRequest, ManageError, OsSecretStore, RegisterEvent, RegistryStore,
    SettingsChange, SiteRecord, SiteState, SiteStatus,
};

/// Handles a `vault.*` method; `None` = not one.
pub(super) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "vault.list" => list(),
        "vault.show" => show(args),
        "vault.lock" => lock(args),
        "vault.lockAll" => lock_all(args),
        "vault.setSettings" => set_settings(args),
        "vault.forgetPassword" => forget_password(args),
        _ => {
            return super::vault_open_policy::handle(rt, method, args)
                .or_else(|| super::vault_keys::handle(rt, method, args))
                .or_else(|| super::vault_compare::handle(rt, method, args))
        }
    })
}

/// The `vault` event for every change of the register (U31-17): state and
/// the reason key of a lock. Registered once per process.
pub(super) fn install_events(rt: &'static Runtime) {
    static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if INSTALLED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    crate::vault::register().subscribe(std::sync::Arc::new(move |event: &RegisterEvent| {
        rt.emit(event_json(event));
    }));
}

pub(super) fn event_json(event: &RegisterEvent) -> Value {
    let state = match event.state {
        SiteState::Unlocked => "unlocked",
        SiteState::Locking => "locking",
        SiteState::Locked | SiteState::Unavailable => "locked",
    };
    json!({
        "type": "vault",
        "site": event.site,
        "state": state,
        "reason": event.reason.and_then(reason_key),
    })
}

/// The five reasons the screens name (U31-10); the desktop-only ones and a
/// drive end carry none.
pub(super) fn reason_key(reason: LockReason) -> Option<&'static str> {
    match reason {
        LockReason::ScreenOff => Some("screen_off"),
        LockReason::Background => Some("background"),
        LockReason::Idle => Some("idle"),
        LockReason::Explicit | LockReason::Removed => Some("manual"),
        LockReason::WorkerEnd => Some("process_end"),
        LockReason::Sleep | LockReason::ScreenLock | LockReason::MountEnd => None,
    }
}

/// U31-13: a vault sync run left conflicts: the notification names the
/// label of the vault and the count, never a file. The job ID carries the
/// prefix `vault:`, so the app opens the comparison of the job
/// (`NavRequest.OpenVaultCompare`) instead of the general conflict list.
pub(super) fn notify_conflicts(job: &crate::syncjobs::SyncJob, count: usize) {
    if count == 0 {
        return;
    }
    let Ok(rt) = Runtime::get() else {
        return;
    };
    let label = label_of(&job.vault_site_a);
    rt.emit(json!({
        "type": "syncProblem",
        "jobId": format!("vault:{}", job.id),
        "title": "Tresor-Sync: Konflikte",
        "text": format!("{count} Konflikte in {label}."),
    }));
}

/// The label of `site` for titles and texts; the site ID when it has none.
pub(crate) fn label_of(site: &str) -> String {
    RegistryStore::open_default()
        .resolve(site)
        .map(|record| record.display_label().to_string())
        .unwrap_or_else(|_| site.to_string())
}

pub(super) fn manage_error(error: ManageError) -> ApiError {
    let kind = match &error {
        ManageError::Invalid(_) | ManageError::Refused(_) => "invalid",
        ManageError::Storage(_) => "network",
        ManageError::Registry(_) => "internal",
        ManageError::InUse(_) => "conflict",
    };
    ApiError::new(kind, error.to_string())
}

fn lock_error(error: LockError) -> ApiError {
    ApiError::new(error.code(), error.to_string())
}

fn store_error(context: &str, error: impl std::fmt::Display) -> ApiError {
    ApiError::internal(format!("{context}: {error}"))
}

/// Where a site lies, as the list shows it: `local`, `connection` (with its
/// name), `share` or `gdrive`.
fn location_kind(location: &str) -> (&'static str, Option<String>) {
    match crate::mobile::location::Loc::parse(location) {
        Ok(loc) if loc.is_local() => ("local", None),
        Ok(loc) => match loc.kind {
            crate::mobile::location::LocKind::Share => ("share", None),
            crate::mobile::location::LocKind::GDrive => ("gdrive", None),
            _ => ("connection", Some(crate::mobile::crumbs::root_label(&loc))),
        },
        Err(_) => ("connection", None),
    }
}

fn site_json(status: &SiteStatus, jobs: &[String]) -> Value {
    let record = &status.record;
    let (kind, connection) = location_kind(&record.location);
    let session = crate::vault::unlocked_session(&record.id);
    let cipher = session
        .as_ref()
        .map(|session| match session.config().combo {
            crate::cryptomator::CipherCombo::SivGcm => "SIV_GCM",
            crate::cryptomator::CipherCombo::SivCtrMac => "SIV_CTRMAC",
        });
    let lock = &record.lock;
    json!({
        "site": record.id,
        "label": record.display_label(),
        "location": record.location,
        "kind": kind,
        "connection": connection,
        "state": status.state.as_str(),
        "replica": record.replica_of.is_some(),
        "format": session.as_ref().map(|_| 8),
        "cipher": cipher,
        "rememberPassword": record.remember_password,
        "autoLockIdleMin": lock.auto_lock_idle_min,
        "lockOnScreenLock": lock.lock_on_screen_lock,
        "lockOnBackgroundMin": lock.lock_on_background_min,
        "syncJobs": jobs,
    })
}

fn job_dirs() -> [std::path::PathBuf; 2] {
    let sync = crate::support_dirs::sync_data_dir();
    [sync.join("jobs"), sync.join("vault-jobs")]
}

/// `vault.list` → `{"vaults":[{"vaultId", "sites":[…]}]}`: sites of one
/// vault (same `jti`) together, a site without an ID alone.
fn list() -> Result<Value, ApiError> {
    let load = RegistryStore::open_default()
        .load()
        .map_err(|error| store_error("Tresorliste lesen", error))?;
    let register = crate::vault::register();
    register.reload_settings(&load.sites);
    let dirs = job_dirs();
    let mut groups: Vec<(Option<String>, Vec<Value>)> = Vec::new();
    for status in register.status(&load.sites) {
        let jobs = crate::vault::jobs_using(&status.record.id, &dirs).unwrap_or_default();
        let id = (!status.record.vault_id.is_empty()).then(|| status.record.vault_id.clone());
        let site = site_json(&status, &jobs);
        match groups
            .iter_mut()
            .find(|(group, _)| id.is_some() && *group == id)
        {
            Some((_, sites)) => sites.push(site),
            None => groups.push((id, vec![site])),
        }
    }
    for (file, reason) in load.broken {
        groups.push((
            None,
            vec![json!({
                "site": file.trim_end_matches(".conf"),
                "label": "",
                "state": "unavailable",
                "stateDetail": reason,
            })],
        ));
    }
    let vaults: Vec<Value> = groups
        .into_iter()
        .map(|(id, sites)| json!({ "vaultId": id, "sites": sites }))
        .collect();
    Ok(json!({ "vaults": vaults }))
}

fn record_of(args: &Value) -> Result<SiteRecord, ApiError> {
    RegistryStore::open_default()
        .resolve(str_arg(args, "site")?)
        .map_err(ApiError::not_found)
}

/// `vault.show {site}`: one site with the recovery key note (T12).
fn show(args: &Value) -> Result<Value, ApiError> {
    let record = record_of(args)?;
    let register = crate::vault::register();
    let status = register
        .status(std::slice::from_ref(&record))
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::not_found("Tresorort ist nicht eingetragen"))?;
    let jobs = crate::vault::jobs_using(&record.id, &job_dirs()).unwrap_or_default();
    let mut site = site_json(&status, &jobs);
    let info = crate::vault::recovery_key_info();
    if let Some(object) = site.as_object_mut() {
        object.insert("recoveryKeySupported".into(), json!(info.supported));
        object.insert("recoveryKeyNote".into(), json!(info.note));
    }
    Ok(site)
}

/// `vault.lock {site?, keepUnsaved?}`: one site or, without `site`, all.
fn lock(args: &Value) -> Result<Value, ApiError> {
    let keep = opt_bool(args, "keepUnsaved", false);
    let register = crate::vault::register();
    let locked = match opt_str(args, "site") {
        None => register.lock_all_explicit(keep).map_err(lock_error)?,
        Some(_) => {
            let record = record_of(args)?;
            let request = LockRequest::new(LockReason::Explicit, keep);
            if register.lock(&record.id, request).map_err(lock_error)? {
                vec![record.id]
            } else {
                Vec::new()
            }
        }
    };
    Ok(json!({ "locked": locked }))
}

fn lock_all(args: &Value) -> Result<Value, ApiError> {
    let keep = opt_bool(args, "keepUnsaved", false);
    let locked = crate::vault::register()
        .lock_all_explicit(keep)
        .map_err(lock_error)?;
    Ok(json!({ "locked": locked }))
}

/// A field that may be absent (keep), `null` (off) or minutes.
fn minutes(args: &Map<String, Value>, key: &str) -> Result<Option<Option<u32>>, ApiError> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => value
            .as_u64()
            .and_then(|minutes| u32::try_from(minutes).ok())
            .map(|minutes| Some(Some(minutes)))
            .ok_or_else(|| invalid(format!("„{key}“ ist eine Minutenzahl oder null."))),
    }
}

/// `vault.setSettings {site, label?, autoLockIdleMin?, lockOnScreenLock?,
/// lockOnBackgroundMin?}`; the register takes the lock settings at once.
fn set_settings(args: &Value) -> Result<Value, ApiError> {
    let object = args
        .as_object()
        .ok_or_else(|| invalid("Argumente fehlen."))?;
    let change = SettingsChange {
        label: opt_str(args, "label").map(str::to_string),
        auto_lock_idle_min: minutes(object, "autoLockIdleMin")?,
        lock_on_screen_lock: object.get("lockOnScreenLock").and_then(Value::as_bool),
        lock_on_background_min: minutes(object, "lockOnBackgroundMin")?,
        ..SettingsChange::default()
    };
    let store = RegistryStore::open_default();
    let record =
        crate::vault::update_settings(&store, str_arg(args, "site")?, &change, &OsSecretStore)
            .map_err(manage_error)?;
    show(&json!({ "site": record.id }))
}

/// `vault.forgetPassword {site}`: deletes the remembered password (S1: the
/// sealed envelope) and clears the flag.
fn forget_password(args: &Value) -> Result<Value, ApiError> {
    let change = SettingsChange {
        remember_password: Some(false),
        ..SettingsChange::default()
    };
    crate::vault::update_settings(
        &RegistryStore::open_default(),
        str_arg(args, "site")?,
        &change,
        &OsSecretStore,
    )
    .map_err(manage_error)?;
    Ok(json!({}))
}
