//! Which roots other apps see, and the vault roots of the Android
//! documents provider (DRV-M8, S2-V5, C3, C6).
//!
//! „Ordner für Apps bereitstellen“ (`docs.settings.*`) releases the device
//! tree, single saved connections, Google Drive and single vaults; nothing
//! is released by default. A root that loses its release loses its open
//! descriptors and its engine at once, like a locked vault.
//!
//! A vault appears as root `vault:<site id>` only while it is unlocked in
//! this app process (the session register, VLT-M4) and the person released
//! it in „Ordner für Apps bereitstellen“. Its engine reads with the vault's
//! `open_read_at` (large files by ranges, no copy) and keeps changes only
//! in the sealed spool of its own spool key (`Vault::SpoolKey` rules of
//! DRV-M7), so no cleartext reaches the disk.
//!
//! The provider is a lock participant: before the key is wiped it publishes
//! what it can; a lock someone asked for without „Änderungen verschlüsselt
//! aufbewahren“ stops when changes would stay unpublished. Otherwise every
//! open descriptor of the vault is revoked (each further call fails with
//! `EACCES`), the engine and the cached listings go, and the root vanishes
//! from the system picker at once (`docs.changed` with `roots`). Texts name
//! counts only, never a name inside the vault.
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::args::invalid;
use super::docs_engines::{self, Source};
use super::docs_ids::{RootId, GDRIVE};
use crate::mobile::runtime::lock;
use crate::mobile::{ApiError, Runtime};
use crate::mount::BackendRoot;

const SETTINGS_FILE: &str = "docs-provider.json";

/// One vault the provider may offer.
pub(super) struct VaultRoot {
    pub(super) site: String,
    pub(super) label: String,
    /// Stored on this phone (`FLAG_LOCAL_ONLY`, assumption A10).
    pub(super) local: bool,
}

/// Every unlocked vault of the vault list, in the order of the list.
pub(super) fn unlocked() -> Vec<VaultRoot> {
    let Ok(load) = crate::vault::RegistryStore::open_default().load() else {
        return Vec::new();
    };
    let register = crate::vault::register();
    load.sites
        .into_iter()
        .filter(|record| register.session(&record.id).is_some())
        .map(|record| VaultRoot {
            local: is_local(&record.location),
            label: record.display_label().to_string(),
            site: record.id,
        })
        .collect()
}

/// Every vault of the list (settings choices), with its state.
pub(super) fn entered() -> Vec<(VaultRoot, bool)> {
    let Ok(load) = crate::vault::RegistryStore::open_default().load() else {
        return Vec::new();
    };
    let register = crate::vault::register();
    load.sites
        .into_iter()
        .map(|record| {
            let unlocked = register.session(&record.id).is_some();
            let root = VaultRoot {
                local: is_local(&record.location),
                label: record.display_label().to_string(),
                site: record.id,
            };
            (root, unlocked)
        })
        .collect()
}

pub(super) fn is_unlocked(site: &str) -> bool {
    crate::vault::register().session(site).is_some()
}

/// A local path on this phone; every remote locator has a scheme.
fn is_local(location: &str) -> bool {
    location.starts_with('/') && !location.starts_with("//")
}

/// The source of the vault engine: the cleartext view of the unlocked site
/// and the sealed storage of the spool key in `mount_dir`. A locked site is
/// `vault_locked`; nothing is opened then.
pub(super) fn source(site: &str, mount_dir: &Path) -> io::Result<Source> {
    let bridge = crate::vault::vault_drive_bridge();
    let backend = bridge
        .open_backend(site)
        .map_err(|error| match error.code() {
            Some(_) => crate::vault::vault_locked(site),
            None => io::Error::other(error.to_string()),
        })?;
    let key = bridge.spool_key(site, mount_dir)?;
    let storage = crate::vault::sealed_mount_storage(&key)?;
    let mut source = docs_engines::source(backend, BackendRoot::parse("/")?, true);
    source.storage = Some(storage);
    Ok(source)
}

/// The lock participant of the provider.
struct Participant {
    on_gone: Arc<dyn Fn(&RootId) + Send + Sync>,
}

impl crate::vault::LockParticipant for Participant {
    fn prepare_lock(&self, site: &str, request: &crate::vault::LockRequest) -> Result<(), String> {
        let root = RootId::Vault(site.to_string());
        let key = docs_engines::EngineKey::Root(root.clone());
        if let Some(slot) = docs_engines::opened(&key) {
            // Open writers publish first: their content is in the spool.
            super::docs_io::flush_root(&root);
            let pending = slot
                .fs
                .prepare_lock()
                .map_or(1, |readiness| readiness.pending);
            if pending > 0 && !request.keep_unsaved && !request.reason.forced() {
                return Err(format!("{pending} Änderungen aus anderen Apps"));
            }
        }
        withdraw(&root, &*self.on_gone);
        Ok(())
    }
}

/// Ends every descriptor and the engine of `root` and tells the picker.
pub(super) fn withdraw(root: &RootId, on_gone: &(dyn Fn(&RootId) + Send + Sync)) {
    super::docs_io::revoke(root);
    let slots = docs_engines::close_root(root);
    // The engines end here, outside the table lock; their journals stay.
    drop(slots);
    on_gone(root);
}

/// Registers the participant and the unlock listener once per process.
/// `on_change` hears of every root that came or went.
pub(super) fn install(on_change: Arc<dyn Fn(&RootId) + Send + Sync>) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let register = crate::vault::register();
    register.add_participant(Arc::new(Participant {
        on_gone: Arc::clone(&on_change),
    }));
    register.subscribe(Arc::new(move |event: &crate::vault::RegisterEvent| {
        let root = RootId::Vault(event.site.clone());
        match event.state {
            crate::vault::SiteState::Unlocked => on_change(&root),
            // A lock that passed no participant (none was open) still
            // removes the root; a revoked root has nothing left to end.
            crate::vault::SiteState::Locked => withdraw(&root, &*on_change),
            crate::vault::SiteState::Locking | crate::vault::SiteState::Unavailable => {}
        }
    }));
}

/// „Ordner für Apps bereitstellen“: what other apps may see.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Settings {
    pub(super) provider: bool,
    pub(super) devices: bool,
    /// Account keys of saved connections and `gdrive`.
    pub(super) connections: Vec<String>,
    /// Site IDs of vaults.
    pub(super) vaults: Vec<String>,
}

fn settings_cell() -> &'static Mutex<Option<Settings>> {
    static SETTINGS: OnceLock<Mutex<Option<Settings>>> = OnceLock::new();
    SETTINGS.get_or_init(|| Mutex::new(None))
}

pub(super) fn settings() -> Settings {
    let mut cell = lock(settings_cell());
    cell.get_or_insert_with(|| {
        let path = crate::support_dirs::app_data_file(SETTINGS_FILE);
        crate::support_dirs::read_private_text(&path, 1 << 20)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    })
    .clone()
}

/// Whether other apps may reach `root` now.
pub(super) fn released(root: &RootId) -> bool {
    let settings = settings();
    settings.provider
        && match root {
            RootId::Devices => settings.devices,
            RootId::GDrive => settings.connections.iter().any(|id| id == GDRIVE),
            RootId::Conn(account) => settings.connections.contains(account),
            RootId::Vault(site) => settings.vaults.contains(site) && is_unlocked(site),
        }
}

pub(super) fn settings_json() -> Value {
    let settings = settings();
    let connections: Vec<Value> = crate::creds::load_connections_checked()
        .unwrap_or_default()
        .iter()
        .filter(|connection| connection.protocol.is_url())
        .map(|connection| json!({ "id": connection.account(), "label": connection.display() }))
        .chain(
            crate::cloud::is_connected(crate::cloud::Provider::GDrive)
                .then(|| json!({ "id": GDRIVE, "label": "Google Drive" })),
        )
        .collect();
    let vaults: Vec<Value> = entered()
        .into_iter()
        .map(|(vault, unlocked)| {
            json!({ "id": vault.site, "label": vault.label, "unlocked": unlocked, "local": vault.local })
        })
        .collect();
    json!({
        "provider": settings.provider,
        "devices": settings.devices,
        "connections": settings.connections,
        "vaults": settings.vaults,
        "choices": { "connections": connections, "vaults": vaults },
    })
}

/// `docs.settings.set {provider?, devices?, connections?, vaults?}`: roots
/// that are no longer released lose their descriptors and engines at once.
pub(super) fn set_settings(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let before = settings();
    let mut next = before.clone();
    if let Some(value) = args.get("provider") {
        next.provider = value
            .as_bool()
            .ok_or_else(|| invalid("provider muss bool sein."))?;
    }
    if let Some(value) = args.get("devices") {
        next.devices = value
            .as_bool()
            .ok_or_else(|| invalid("devices muss bool sein."))?;
    }
    for (key, list) in [
        ("connections", &mut next.connections),
        ("vaults", &mut next.vaults),
    ] {
        if args.get(key).is_some() {
            *list = super::args::string_list(args, key)?;
            list.sort();
            list.dedup();
        }
    }
    let path = crate::support_dirs::app_data_file(SETTINGS_FILE);
    let text =
        serde_json::to_string(&next).map_err(|error| ApiError::internal(error.to_string()))?;
    crate::support_dirs::write_private_atomic(&path, text.as_bytes())
        .map_err(|error| ApiError::from(error).context("Einstellungen speichern"))?;
    *lock(settings_cell()) = Some(next.clone());
    for root in withdrawn(&before, &next) {
        withdraw(&root, &|root: &RootId| super::docs::forget_listings(root));
    }
    rt.emit(
        json!({ "type": "docs.changed", "roots": true, "documents": [], "commitFailed": false }),
    );
    Ok(settings_json())
}

/// Roots `before` released and `next` does not.
fn withdrawn(before: &Settings, next: &Settings) -> Vec<RootId> {
    let mut roots = Vec::new();
    let all_off = before.provider && !next.provider;
    if before.devices && (all_off || !next.devices) {
        roots.push(RootId::Devices);
    }
    for id in &before.connections {
        if all_off || !next.connections.contains(id) {
            roots.push(if id == GDRIVE {
                RootId::GDrive
            } else {
                RootId::Conn(id.clone())
            });
        }
    }
    for site in &before.vaults {
        if all_off || !next.vaults.contains(site) {
            roots.push(RootId::Vault(site.clone()));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_task_docs_withdrawn_roots_lose_their_release() {
        let before = Settings {
            provider: true,
            devices: true,
            connections: vec!["sftp://a@h:22/".into(), GDRIVE.into()],
            vaults: vec!["0123456789abcdef0123456789abcdef".into()],
        };
        let mut next = before.clone();
        next.connections.retain(|id| id != GDRIVE);
        assert_eq!(withdrawn(&before, &next), vec![RootId::GDrive]);
        next.provider = false;
        let all = withdrawn(&before, &next);
        assert_eq!(all.len(), 4);
        assert!(all.contains(&RootId::Devices));
        assert!(all.contains(&RootId::Vault("0123456789abcdef0123456789abcdef".into())));
        assert!(withdrawn(&before, &before).is_empty());
    }

    #[test]
    fn drive_task_docs_nothing_released_by_default() {
        let settings = Settings::default();
        assert!(!settings.provider && !settings.devices);
        assert!(settings.connections.is_empty() && settings.vaults.is_empty());
        assert!(is_local("/storage/emulated/0/Tresor"));
        assert!(!is_local("sftp://a@h:22/tresor") && !is_local("//server/share"));
    }
}
