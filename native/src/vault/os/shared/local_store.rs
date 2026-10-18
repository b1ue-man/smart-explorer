//! What se keeps about places inside vaults, and where vault cleartext
//! appears on this device (vault plan C4, VLT-M6):
//!
//! - K13/K14: favourites, recent places and folder sort settings inside a
//!   vault are stored globally only as the vault root `vault://<site-id>/`;
//!   the places inside are sealed with the purpose key `local-store` in
//!   `<AD>/vaults/<site-id>.local` and come back only while the vault is
//!   unlocked. A place that cannot be sealed now (vault locked, no worker)
//!   is never written in cleartext; the sealed list stored before stays.
//! - K09/K11/K21: the folders and drive roots where vault drives appear,
//!   published by the drive layer, and the fixed Linux folder of vault
//!   drives (`$XDG_RUNTIME_DIR/smart-explorer/Tresore`, S2 C2) as a whole.
//! - K20: the site of an opened vault backend by its state identity.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};

use crate::vault::core::klartext_policy::{root_place, root_site, split_place, within};
use crate::vault::core::locator::{is_site_id, vault_location};
use crate::vault::core::sealed::SealPurpose;
use crate::vfs::{Backend, Scheme};

use super::registry_store::{RegistryStore, VAULTS_DIR};
use super::session::{hosts_register, register};

/// Read limit of one `<site-id>.local`.
const MAX_LOCAL_BYTES: u64 = 16 * 1024 * 1024;

/// Seals and unseals texts of a site's purpose key.
pub(crate) trait SiteSealing: Send + Sync {
    fn seal(&self, site: &str, purpose: SealPurpose, items: &[&str])
        -> Result<Vec<String>, String>;
    fn unseal(
        &self,
        site: &str,
        purpose: SealPurpose,
        items: &[String],
    ) -> Result<Vec<String>, String>;
}

/// The register of this process when it holds the keys (worker, Android
/// app), else the worker over IPC (`Vault::Seal`, `Vault::Unseal`).
pub(crate) struct ProcessSealing;

fn wire(purpose: SealPurpose) -> Result<crate::daemon::SealPurposeWire, String> {
    match purpose {
        SealPurpose::LocalStore => Ok(crate::daemon::SealPurposeWire::LocalStore),
        SealPurpose::LogFields => Ok(crate::daemon::SealPurposeWire::LogFields),
        SealPurpose::SpoolWrap => Err("spool-wrap versiegelt keine Texte".into()),
    }
}

impl SiteSealing for ProcessSealing {
    fn seal(
        &self,
        site: &str,
        purpose: SealPurpose,
        items: &[&str],
    ) -> Result<Vec<String>, String> {
        if hosts_register() {
            return register()
                .seal_texts(site, purpose, items)
                .map_err(|error| error.to_string());
        }
        crate::daemon::seal_vault_items(site, wire(purpose)?, items).map_err(|error| error.message)
    }

    fn unseal(
        &self,
        site: &str,
        purpose: SealPurpose,
        items: &[String],
    ) -> Result<Vec<String>, String> {
        if hosts_register() {
            return register()
                .unseal_texts(site, purpose, items)
                .map(|texts| texts.iter().map(|text| text.as_str().to_string()).collect())
                .map_err(|error| error.to_string());
        }
        crate::daemon::unseal_vault_items(site, wire(purpose)?, items)
            .map(|texts| texts.iter().map(|text| text.expose().to_string()).collect())
            .map_err(|error| error.message)
    }
}

/// A list of places kept per vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceList {
    Favorites,
    Recent,
}

/// `<site-id>.local`: sealed paths inside the vault.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    favorites: Vec<String>,
    #[serde(default)]
    recent: Vec<String>,
    #[serde(default)]
    dir_sort: Vec<(String, bool)>,
}

/// The sealed place lists of the vault sites.
pub(crate) struct LocalStore {
    dir: PathBuf,
}

impl LocalStore {
    /// `<AD>/vaults`.
    pub(crate) fn open_default() -> Self {
        Self::at(crate::support_dirs::app_data_dir().join(VAULTS_DIR))
    }

    pub(crate) fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(crate) fn path(&self, site: &str) -> PathBuf {
        self.dir.join(format!("{site}.local"))
    }

    fn read(&self, site: &str) -> Stored {
        crate::support_dirs::read_private_text(&self.path(site), MAX_LOCAL_BYTES)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, site: &str, stored: &Stored) -> Result<(), String> {
        let text = serde_json::to_string(stored).map_err(|error| error.to_string())?;
        crate::support_dirs::write_private_atomic(&self.path(site), text.as_bytes())
            .map_err(|error| format!("Tresor-Ortsdaten schreiben: {error}"))
    }

    /// The places of `list` inside `site` (cleartext paths, `/` = root).
    pub(crate) fn places(
        &self,
        sealing: &dyn SiteSealing,
        site: &str,
        list: PlaceList,
    ) -> Result<Vec<String>, String> {
        if !is_site_id(site) {
            return Err("Tresorort-ID erwartet".into());
        }
        let stored = self.read(site);
        let sealed = match list {
            PlaceList::Favorites => &stored.favorites,
            PlaceList::Recent => &stored.recent,
        };
        if sealed.is_empty() {
            return Ok(Vec::new());
        }
        sealing.unseal(site, SealPurpose::LocalStore, sealed)
    }

    /// Replaces the places of `list` inside `site`; nothing is written when
    /// they cannot be sealed.
    pub(crate) fn set_places(
        &self,
        sealing: &dyn SiteSealing,
        site: &str,
        list: PlaceList,
        places: &[String],
    ) -> Result<(), String> {
        if !is_site_id(site) {
            return Err("Tresorort-ID erwartet".into());
        }
        let items: Vec<&str> = places.iter().map(String::as_str).collect();
        let sealed = sealing.seal(site, SealPurpose::LocalStore, &items)?;
        let mut stored = self.read(site);
        match list {
            PlaceList::Favorites => stored.favorites = sealed,
            PlaceList::Recent => stored.recent = sealed,
        }
        self.write(site, &stored)
    }

    /// The folder sort settings inside `site`.
    pub(crate) fn dir_sort(
        &self,
        sealing: &dyn SiteSealing,
        site: &str,
    ) -> Result<Vec<(String, bool)>, String> {
        let stored = self.read(site);
        let sealed: Vec<String> = stored
            .dir_sort
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        if sealed.is_empty() {
            return Ok(Vec::new());
        }
        let paths = sealing.unseal(site, SealPurpose::LocalStore, &sealed)?;
        Ok(paths
            .into_iter()
            .zip(stored.dir_sort.iter().map(|(_, first)| *first))
            .collect())
    }

    fn set_dir_sort(
        &self,
        sealing: &dyn SiteSealing,
        site: &str,
        entries: &[(String, bool)],
    ) -> Result<(), String> {
        let items: Vec<&str> = entries.iter().map(|(path, _)| path.as_str()).collect();
        let sealed = sealing.seal(site, SealPurpose::LocalStore, &items)?;
        let mut stored = self.read(site);
        stored.dir_sort = sealed
            .into_iter()
            .zip(entries.iter().map(|(_, first)| *first))
            .collect();
        self.write(site, &stored)
    }

    /// Sites with a `.local` file.
    fn sites(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".local")
                    .filter(|site| is_site_id(site))
                    .map(str::to_string)
            })
            .collect()
    }
}

/// The site ID behind `site` as written (ID, ID prefix or label).
fn site_id(site: &str) -> Option<String> {
    if is_site_id(site) {
        return Some(site.to_string());
    }
    RegistryStore::open_default()
        .resolve(site)
        .ok()
        .map(|record| record.id)
}

/// The global form of `places` (K13, K14): a place inside a vault becomes
/// the vault root (once, where its first place stood); the places inside
/// are stored sealed per site.
pub(crate) fn store_places(
    store: &LocalStore,
    sealing: &dyn SiteSealing,
    list: PlaceList,
    places: &[String],
) -> Vec<String> {
    let mut global: Vec<String> = Vec::with_capacity(places.len());
    let mut inside: Vec<(String, Vec<String>)> = Vec::new();
    for place in places {
        let Some((written, path)) = split_place(place) else {
            // Every other place stays exactly as it was.
            global.push(place.clone());
            continue;
        };
        let Some(site) = site_id(&written) else {
            let root = root_place(&written);
            if !global.contains(&root) {
                global.push(root);
            }
            continue;
        };
        let root = root_place(&site);
        if !global.contains(&root) {
            global.push(root);
        }
        match inside.iter_mut().find(|(known, _)| *known == site) {
            Some((_, paths)) if !paths.contains(&path) => paths.push(path),
            Some(_) => {}
            None => inside.push((site, vec![path])),
        }
    }
    for (site, paths) in inside {
        // A vault that cannot seal now keeps its stored list.
        let _ = store.set_places(sealing, &site, list, &paths);
    }
    global
}

/// The places for display: each vault root of `global` is replaced by the
/// places stored inside it while the vault is unlocked; locked, it stays.
pub(crate) fn expand_places(
    store: &LocalStore,
    sealing: &dyn SiteSealing,
    list: PlaceList,
    global: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(global.len());
    for place in global {
        if split_place(place).is_none() {
            out.push(place.clone());
            continue;
        }
        let inside = root_site(place).and_then(|site| {
            store
                .places(sealing, &site, list)
                .ok()
                .filter(|paths| !paths.is_empty())
                .map(|paths| (site, paths))
        });
        let expanded = match inside {
            Some((site, paths)) => paths
                .iter()
                .map(|path| vault_location(&site, path))
                .collect(),
            None => vec![place.clone()],
        };
        for place in expanded {
            if !out.contains(&place) {
                out.push(place);
            }
        }
    }
    out
}

/// The global folder sort settings: keys inside a vault are stored sealed
/// per site, vault roots and every other key stay.
pub(crate) fn store_dir_sort(
    store: &LocalStore,
    sealing: &dyn SiteSealing,
    map: &HashMap<String, bool>,
) -> HashMap<String, bool> {
    let mut global = HashMap::new();
    let mut inside: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (key, first) in map {
        match split_place(key).and_then(|(written, path)| Some((site_id(&written)?, path))) {
            Some((site, path)) if path != "/" => {
                inside.entry(site).or_default().push((path, *first))
            }
            Some((site, _)) => {
                global.insert(root_place(&site), *first);
            }
            None if split_place(key).is_some() => {}
            None => {
                global.insert(key.clone(), *first);
            }
        }
    }
    for (site, mut entries) in inside {
        entries.sort();
        let _ = store.set_dir_sort(sealing, &site, &entries);
    }
    global
}

/// `global` plus the sort settings inside every vault that unseals now.
pub(crate) fn expand_dir_sort(
    store: &LocalStore,
    sealing: &dyn SiteSealing,
    global: HashMap<String, bool>,
) -> HashMap<String, bool> {
    let mut map = global;
    for site in store.sites() {
        if let Ok(entries) = store.dir_sort(sealing, &site) {
            for (path, first) in entries {
                map.insert(vault_location(&site, &path), first);
            }
        }
    }
    map
}

fn mounts() -> &'static Mutex<Vec<(String, PathBuf)>> {
    static MOUNTS: OnceLock<Mutex<Vec<(String, PathBuf)>>> = OnceLock::new();
    MOUNTS.get_or_init(Default::default)
}

/// A vault drive of `site` appeared at `path` (folder or drive root).
pub(crate) fn publish_vault_mount(site: &str, path: &Path) {
    let mut known = mounts().lock().unwrap_or_else(PoisonError::into_inner);
    if !known
        .iter()
        .any(|(known_site, known_path)| known_site == site && known_path == path)
    {
        known.push((site.to_string(), path.to_path_buf()));
    }
}

/// The vault drive of `site` at `path` ended.
pub(crate) fn withdraw_vault_mount(site: &str, path: &Path) {
    mounts()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|(known_site, known_path)| !(known_site == site && known_path == path));
}

/// Every place where vault cleartext appears as files: published vault
/// drives with their site, and the Linux folder of all vault drives.
pub(crate) fn vault_mount_roots() -> Vec<(Option<String>, PathBuf)> {
    let mut roots: Vec<(Option<String>, PathBuf)> = mounts()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(site, path)| (Some(site.clone()), path.clone()))
        .collect();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        roots.push((
            None,
            PathBuf::from(runtime)
                .join("smart-explorer")
                .join("Tresore"),
        ));
    }
    roots
}

/// `Some(site)` when `path` lies in a vault drive (`Some(None)`: a vault
/// drive whose site is not known here).
pub(crate) fn vault_mount_of(path: &Path) -> Option<Option<String>> {
    vault_mount_roots()
        .into_iter()
        .filter(|(_, root)| within(path, root))
        .max_by_key(|(site, _)| site.is_some())
        .map(|(site, _)| site)
}

/// Whether a path or location shows vault cleartext: a `vault://` or
/// `vault-store://` location, or a path in a vault drive (K11, K12).
pub(crate) fn shows_vault(path: &str) -> bool {
    VaultPlaces::now().shows_vault(path)
}

/// The vault drives of one moment, for checking many paths (a folder index
/// of up to a million paths reads them once).
pub(crate) struct VaultPlaces {
    roots: Vec<PathBuf>,
}

impl VaultPlaces {
    pub(crate) fn now() -> Self {
        Self {
            roots: vault_mount_roots()
                .into_iter()
                .map(|(_, root)| root)
                .collect(),
        }
    }

    /// `shows_vault` against this reading.
    pub(crate) fn shows_vault(&self, path: &str) -> bool {
        crate::vault::core::locator::is_vault_locator(path)
            || self.roots.iter().any(|root| within(Path::new(path), root))
    }
}

fn backends() -> &'static Mutex<HashMap<String, String>> {
    static BACKENDS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    BACKENDS.get_or_init(Default::default)
}

/// Remembers the site of an opened vault backend (K20: transfer logs seal
/// their fields with its key).
pub(crate) fn note_vault_backend(backend: &dyn Backend, site: &str) {
    if backend.scheme() == Scheme::Vault {
        backends()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(backend.state_identity(), site.to_string());
    }
}

/// `Some(site)` for a vault backend opened here, `Some(None)` for a vault
/// backend of unknown site, `None` for every other backend.
pub(crate) fn vault_site_of(backend: &dyn Backend) -> Option<Option<String>> {
    if backend.scheme() != Scheme::Vault {
        return None;
    }
    Some(
        backends()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&backend.state_identity())
            .cloned(),
    )
}

/// Seals log fields of `site` (K20, K21) in this process or the worker.
pub(crate) fn seal_log_fields(site: &str, items: &[&str]) -> Result<Vec<String>, String> {
    ProcessSealing.seal(site, SealPurpose::LogFields, items)
}

#[cfg(test)]
#[path = "local_store_tests.rs"]
mod tests;
