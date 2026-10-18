//! The second side of a vault sync job without a folder setup (vault plan
//! C5.2) and the job itself (C7.1 T15: `se vault sync add`, „Synchronisieren…“,
//! `vault.syncCreate`):
//!
//! - Z-a (default): a copy se creates and keeps, entered as a site of kind
//!   `replica` with `seeding=true`; its first complete run fills it (C5.9).
//! - Z-b: an entered site of the same vault (same `jti`).
//! - Z-c: a new empty folder at any se location, entered and seeded like
//!   a copy.
//!
//! The default copies lie next to the app data folder, not in it: every
//! walk of se leaves the app data folder out as its own files
//! (`bisync::snapshot_policy::own_path`), so a copy there could never sync.
use std::path::{Path, PathBuf};

use crate::syncjobs::SyncJob;
use crate::vault::core::registry_types::{new_site_id, SiteKind, SiteRecord};
use crate::vfs::Backend;

use super::open::ResolveLocation;
use super::registry_store::RegistryStore;
use super::unlock::{read_key_files, UnlockError};

/// Folder of the default copies next to `<data home>/smart_explorer`.
pub(crate) const REPLICAS_DIR: &str = "smart_explorer-vault-replicas";

/// `<data home>/smart_explorer-vault-replicas` (desktop: next to the app
/// data folder; Android: in `filesDir`, private to the app).
pub(crate) fn default_replicas_root() -> PathBuf {
    let app = crate::support_dirs::app_data_dir();
    match app.parent() {
        Some(parent) => parent.join(REPLICAS_DIR),
        None => app.with_file_name(REPLICAS_DIR),
    }
}

/// Where the second side of a new vault job comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyTarget<'a> {
    /// A new copy below the replica folder (Z-a).
    Replica,
    /// An entered site of the same vault, by ID, ID prefix or label (Z-b).
    Site(&'a str),
    /// A new or empty folder at an se location (Z-c).
    Location(&'a str),
}

/// A created vault job and its second site.
#[derive(Clone, Debug)]
pub(crate) struct VaultJobCreated {
    pub(crate) job: SyncJob,
    pub(crate) site_a: SiteRecord,
    pub(crate) site_b: SiteRecord,
    /// The second site was entered by this call (Z-a, Z-c).
    pub(crate) created_site: bool,
}

/// What `create_vault_job` needs besides the vault list.
pub(crate) struct JobPlace<'a> {
    /// Folder of the default copies (`default_replicas_root`).
    pub(crate) replicas_root: &'a Path,
    pub(crate) resolve: ResolveLocation<'a>,
    /// Whether a job ID is taken in `jobs/` or `vault-jobs/` (C5.8).
    pub(crate) id_taken: &'a dyn Fn(&str) -> bool,
    /// Saves the new job (`syncjobs::upsert`).
    pub(crate) save: &'a dyn Fn(&SyncJob) -> Result<(), String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The vault ID (`jti`) stored at a site's location, read without a key.
fn stored_vault_id(record: &SiteRecord, resolve: ResolveLocation<'_>) -> Result<String, String> {
    let (inner, root) = resolve(&record.location)?;
    vault_id_at(inner.as_ref(), &root)?
        .ok_or_else(|| format!("Kein Tresor bei „{}“", record.display_label()))
}

/// The `jti` of `vault.cryptomator` in `root`; `None` without one.
pub(crate) fn vault_id_at(inner: &dyn Backend, root: &str) -> Result<Option<String>, String> {
    match read_key_files(inner, root) {
        Ok(files) => Ok(files.config.and_then(|config| config.alleged_vault_id())),
        Err(UnlockError::Unreachable(cause)) => {
            Err(format!("Speicherort nicht erreichbar: {cause}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

/// A label of the copy not used by another site.
fn copy_label(store: &RegistryStore, original: &SiteRecord) -> Result<String, String> {
    let load = store
        .load()
        .map_err(|error| format!("Tresorliste lesen: {error}"))?;
    let base = format!("{} (Kopie)", original.display_label());
    let mut label = base.clone();
    let mut number = 2;
    while load.sites.iter().any(|site| site.label == label) {
        label = format!("{base} {number}");
        number += 1;
    }
    Ok(label)
}

fn copy_record(
    store: &RegistryStore,
    original: &SiteRecord,
    jti: &str,
    location: &str,
) -> Result<SiteRecord, String> {
    let id = new_site_id()?;
    let label = copy_label(store, original)?;
    let mut record = SiteRecord::new(&id, &label, location, jti, now_ms(), store.defaults());
    record.kind = SiteKind::Replica;
    record.replica_of = Some(original.id.clone());
    record.seeding = true;
    Ok(record)
}

/// Z-c: the folder must be new or empty; a folder holding this vault is
/// entered as it is (no seeding), one holding another vault is refused.
fn prepare_location(
    location: &str,
    jti: &str,
    resolve: ResolveLocation<'_>,
) -> Result<bool, String> {
    let (inner, root) = resolve(location)?;
    match inner.stat(&root) {
        Ok(meta) if meta.is_dir => {
            if let Some(found) = vault_id_at(inner.as_ref(), &root)? {
                return if found == jti {
                    Ok(false)
                } else {
                    Err("Am Zielort liegt ein anderer Tresor".into())
                };
            }
            let entries = inner.list_dir(&root).map_err(|error| error.to_string())?;
            if entries.is_empty() {
                Ok(true)
            } else {
                Err("Der Zielordner ist nicht leer: einen leeren oder neuen Ordner wählen".into())
            }
        }
        Ok(_) => Err("Am Zielort liegt eine Datei".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            inner.mkdir_all(&root).map_err(|error| error.to_string())?;
            Ok(true)
        }
        Err(error) => Err(format!("Speicherort nicht erreichbar: {error}")),
    }
}

/// Creates the second side (Z-a, Z-b or Z-c) and the vault job between
/// `original` and it. A failure removes what this call entered or created.
pub(crate) fn create_vault_job(
    store: &RegistryStore,
    original: &str,
    target: CopyTarget<'_>,
    name: Option<&str>,
    place: &JobPlace<'_>,
) -> Result<VaultJobCreated, String> {
    let site_a = store.resolve(original)?;
    let jti = if site_a.vault_id.is_empty() {
        stored_vault_id(&site_a, place.resolve)?
    } else {
        site_a.vault_id.clone()
    };
    let mut made_folder: Option<PathBuf> = None;
    let (site_b, created_site) = match target {
        CopyTarget::Replica => {
            let mut record = copy_record(store, &site_a, &jti, "")?;
            let folder = place.replicas_root.join(&record.id);
            std::fs::create_dir_all(&folder).map_err(|error| format!("Kopie anlegen: {error}"))?;
            made_folder = Some(folder.clone());
            record.location = forward(&folder);
            (record, true)
        }
        CopyTarget::Site(query) => {
            let record = store.resolve(query)?;
            if record.id == site_a.id {
                return Err("Beide Seiten sind derselbe Tresorort".into());
            }
            let other = if record.vault_id.is_empty() {
                stored_vault_id(&record, place.resolve)?
            } else {
                record.vault_id.clone()
            };
            if other != jti {
                return Err("Der gewählte Tresorort enthält einen anderen Tresor".into());
            }
            (record, false)
        }
        CopyTarget::Location(location) => {
            let seeding = prepare_location(location.trim(), &jti, place.resolve)?;
            let mut record = copy_record(store, &site_a, &jti, location.trim())?;
            record.seeding = seeding;
            (record, true)
        }
    };
    let undo = |error: String| {
        if created_site {
            let _ = store.remove(&site_b.id);
        }
        if let Some(folder) = &made_folder {
            let _ = std::fs::remove_dir(folder);
        }
        error
    };
    if created_site {
        store.add(&site_b).map_err(&undo)?;
    }
    let label = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Tresor {}", site_a.display_label()));
    let mut job = SyncJob::new_vault(label, &site_a.id, &site_b.id);
    // IDs are unique over `jobs/` and `vault-jobs/` (C5.8).
    while (place.id_taken)(&job.id) {
        job.id.push('0');
    }
    job.validate().map_err(&undo)?;
    (place.save)(&job).map_err(&undo)?;
    Ok(VaultJobCreated {
        job,
        site_a,
        site_b,
        created_site,
    })
}
