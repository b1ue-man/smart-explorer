//! Text and JSON of `se vault` (vault plan C7.3, U26-10 to U26-12): the
//! JSON keys are the keys of C7.3, `*_ms` are integers, missing values are
//! `null` and never left out; text is tab-separated with `clean()` on every
//! name. Also the resolution of `<SITE>` with the messages of the CLI, the
//! texts of the structured vault codes and the summary `se mcp status` shows.
use std::collections::HashMap;
use std::io::Read;

use base64::Engine;
use serde_json::{json, Value};

use crate::daemon::{CompareItemWire, VaultIpcError};
use crate::mount::{MountSource, MountStatus};
use crate::vault::{RegistryStore, SiteRecord};

/// `JSON` pretty printed.
pub(crate) fn pretty(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| error.to_string())
}

/// Tabs and line breaks of a name become spaces (one row, one line).
pub(crate) fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

/// The entered site `<SITE>` names (ID, ID prefix of at least four digits
/// or label), with the messages of the CLI.
pub(crate) fn site_of(text: &str) -> Result<SiteRecord, String> {
    let text = text.trim();
    let load = RegistryStore::open_default()
        .load()
        .map_err(|error| format!("Tresorliste lesen: {error}"))?;
    if let Ok(site) = crate::vault::resolve_in(&load.sites, text) {
        return Ok(site);
    }
    let lower = text.to_ascii_lowercase();
    let prefix = lower.len() >= 4 && lower.bytes().all(|byte| byte.is_ascii_hexdigit());
    let hits: Vec<&SiteRecord> = load
        .sites
        .iter()
        .filter(|site| site.label == text || (prefix && site.id.starts_with(&lower)))
        .collect();
    if hits.len() > 1 {
        let named: Vec<String> = hits
            .iter()
            .map(|site| {
                format!(
                    "{} {}",
                    site.id.get(..8).unwrap_or(&site.id),
                    clean(site.display_label())
                )
            })
            .collect();
        return Err(format!(
            "Mehrdeutig: {} Tresore passen zu „{text}“ ({})",
            hits.len(),
            named.join(", ")
        ));
    }
    Err(format!("Unbekannter Tresor: {text}"))
}

/// The text of a refused vault request: the text of its structured code
/// when it has one (`<Tresor>` stands for `site`), else the message.
pub(crate) fn vault_error(error: &VaultIpcError, site: &str) -> String {
    match error
        .code
        .as_deref()
        .and_then(crate::vault::error_text_for_code)
    {
        Some(text) => text.replace("<Tresor>", site),
        None => error.message.clone(),
    }
}

/// What the status of the worker and the drives say about the sites.
pub(crate) struct Context {
    states: HashMap<String, (String, Option<u32>)>,
    mounts: HashMap<String, String>,
    dirs: Vec<std::path::PathBuf>,
}

/// Reads the worker's status and the drives of the vaults; neither starts
/// the worker (no worker means every vault is locked).
pub(crate) fn context() -> Context {
    let mut states = HashMap::new();
    let mut mounts = HashMap::new();
    // Without a running worker no vault is unlocked and no drive runs, and
    // asking for the drives would start it.
    if let Ok(Some(status)) = crate::daemon::vault_status() {
        for site in status.sites {
            let state = match site.state.as_str() {
                "locking" => "unlocked".to_string(),
                other => other.to_string(),
            };
            states.insert(site.site, (state, site.shortening_threshold));
        }
        for snapshot in crate::daemon::list_mounts().unwrap_or_default() {
            let MountSource::Vault { site_id } = &snapshot.config.source else {
                continue;
            };
            let at = match &snapshot.status {
                MountStatus::Mounted { drive } => drive.to_string(),
                MountStatus::MountedAt { path } => path.to_string_lossy().into_owned(),
                _ => continue,
            };
            mounts.insert(site_id.clone(), at);
        }
    }
    let sync = crate::support_dirs::sync_data_dir();
    Context {
        states,
        mounts,
        dirs: vec![sync.join("jobs"), sync.join("vault-jobs")],
    }
}

impl Context {
    pub(crate) fn state_of(&self, site: &str) -> &str {
        self.states
            .get(site)
            .map_or("locked", |(state, _)| state.as_str())
    }

    pub(crate) fn threshold_of(&self, site: &str) -> Option<u32> {
        self.states.get(site).and_then(|(_, threshold)| *threshold)
    }

    pub(crate) fn mounted_of(&self, site: &str) -> Option<&str> {
        self.mounts.get(site).map(String::as_str)
    }
}

/// Format and cipher of the vault at `location` from the unverified part
/// of `vault.cryptomator` (the claims of its token); `None` when it cannot
/// be read, which is no error here.
fn config_claims(location: &str) -> (Option<i64>, Option<String>) {
    let Ok((backend, root)) = crate::connect::resolve_endpoint(location) else {
        return (None, None);
    };
    let path = format!("{}/vault.cryptomator", root.trim_end_matches('/'));
    let Ok(reader) = backend.open_read(&path) else {
        return (None, None);
    };
    let mut token = String::new();
    if reader.take(64 * 1024).read_to_string(&mut token).is_err() {
        return (None, None);
    }
    let payload = token.trim().split('.').nth(1).and_then(|part| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part)
            .ok()
    });
    let claims: Option<Value> = payload.and_then(|bytes| serde_json::from_slice(&bytes).ok());
    match claims {
        Some(claims) => (
            claims.get("format").and_then(Value::as_i64),
            claims
                .get("cipherCombo")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        None => (None, None),
    }
}

/// One site as `list` and `show` show it (C7.3 keys).
pub(crate) fn site_value(record: &SiteRecord, context: &Context) -> Value {
    let (format, cipher) = config_claims(&record.location);
    let jobs = crate::vault::jobs_using(&record.id, &context.dirs).unwrap_or_default();
    json!({
        "site": record.id,
        "label": record.display_label(),
        "location": record.location,
        "kind": record.kind.as_str(),
        "state": context.state_of(&record.id),
        "format": format,
        "cipher": cipher,
        "mounted": context.mounted_of(&record.id),
        "remember_password": record.remember_password,
        "auto_lock_idle_min": record.lock.auto_lock_idle_min,
        "lock_on_sleep": record.lock.lock_on_sleep,
        "lock_on_screen_lock": record.lock.lock_on_screen_lock,
        "lock_on_background_min": record.lock.lock_on_background_min,
        "sync_jobs": jobs,
    })
}

/// `key<TAB>value` lines of an object, in the order of its keys.
pub(crate) fn key_lines(value: &Value) -> Vec<String> {
    let Some(fields) = value.as_object() else {
        return Vec::new();
    };
    fields
        .iter()
        .map(|(key, field)| {
            let text = match field {
                Value::String(text) => text.clone(),
                Value::Null => "-".to_string(),
                Value::Array(items) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map_or_else(|| item.to_string(), str::to_string)
                    })
                    .collect::<Vec<_>>()
                    .join(","),
                other => other.to_string(),
            };
            clean(&format!("{key}\t{text}"))
        })
        .collect()
}

/// One entry as the JSON of `compare` and `check` shows it.
pub(crate) fn item_value(item: &CompareItemWire) -> Value {
    let side = |side: &crate::daemon::CompareSideWire| {
        json!({
            "exists": side.exists,
            "size": side.size,
            "mtime_ms": side.mtime_ms,
            "location": side.location,
        })
    };
    json!({
        "item": item.item,
        "group": item.group,
        "kind": item.kind,
        "path": (!item.path.is_empty()).then_some(item.path.as_str()),
        "a": side(&item.a),
        "b": side(&item.b),
        "actions": item.actions,
        "finding": item.finding,
        "note": item.note,
    })
}

/// `+:<size>` / `-:?`: the state of one side in a text row.
pub(crate) fn side_text(side: &crate::daemon::CompareSideWire) -> String {
    format!(
        "{}:{}",
        if side.exists { '+' } else { '-' },
        side.size
            .map_or_else(|| "?".to_string(), |size| size.to_string())
    )
}

/// The summary `se mcp status` shows under `vaults` (U26-13): numbers only,
/// never a path, a name or a count of conflicts (those need an unlocked
/// vault and a comparison). The worker is asked without being started.
pub(crate) fn vaults_summary() -> Value {
    let entered = RegistryStore::open_default()
        .load()
        .map_or(0, |load| load.sites.len());
    let jobs = crate::syncjobs::load_report().map_or(0, |report| {
        report.jobs.iter().filter(|job| job.is_vault()).count()
    });
    let unlocked = match crate::daemon::vault_status() {
        Ok(Some(status)) => status
            .sites
            .iter()
            .filter(|site| matches!(site.state.as_str(), "unlocked" | "locking"))
            .count(),
        _ => 0,
    };
    json!({
        "entered": entered,
        "unlocked": unlocked,
        "locked": entered.saturating_sub(unlocked),
        "jobs": jobs,
        "full_list": "se tool with [\"vault\",\"list\",\"--json\"]",
    })
}
