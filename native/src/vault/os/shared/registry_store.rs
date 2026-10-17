//! The vault list on disk (C2.1): one `<site-id>.conf` per vault site in
//! `<AD>/vaults/`, private and replaced atomically. Additions run under a
//! process mutex and a file lock (`vaults.lock`), so a vault location is
//! entered once even when two processes add it at the same time.
//!
//! Loading is tolerant: an entry that cannot be read is reported as broken
//! and never hides the others.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::vault::core::locator::{is_id_prefix, is_site_id};
use crate::vault::core::registry_types::{
    location_key, site_file_name, site_id_of_file, LockSettings, SiteRecord,
};

/// Folder of the vault list below the app data folder.
pub(crate) const VAULTS_DIR: &str = "vaults";
const LOCK_FILE: &str = "vaults.lock";
/// Read limit of one entry; a valid entry has well below 4 KiB.
const MAX_SITE_BYTES: u64 = 64 * 1024;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// The vault list in one folder.
pub(crate) struct RegistryStore {
    dir: PathBuf,
    defaults: LockSettings,
}

/// What `load` found: the sites (oldest first) and the entries that could
/// not be read, as (file name, reason).
#[derive(Debug, Default)]
pub(crate) struct RegistryLoad {
    pub(crate) sites: Vec<SiteRecord>,
    pub(crate) broken: Vec<(String, String)>,
}

struct WriteGuard {
    _file: std::fs::File,
    _process: MutexGuard<'static, ()>,
}

impl RegistryStore {
    /// `<AD>/vaults` with this platform's lock defaults.
    pub(crate) fn open_default() -> Self {
        Self::at(
            crate::support_dirs::app_data_dir().join(VAULTS_DIR),
            super::platform_lock_defaults(),
        )
    }

    pub(crate) fn at(dir: PathBuf, defaults: LockSettings) -> Self {
        Self { dir, defaults }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Lock settings of entries that do not carry them, and of new sites.
    pub(crate) fn defaults(&self) -> LockSettings {
        self.defaults
    }

    pub(crate) fn load(&self) -> io::Result<RegistryLoad> {
        let mut load = RegistryLoad::default();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(load),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(id) = site_id_of_file(name) else {
                continue;
            };
            match self.read_site(&entry.path()) {
                Ok(site) if site.id == id => load.sites.push(site),
                Ok(_) => load
                    .broken
                    .push((name.to_string(), "ID passt nicht zum Dateinamen".into())),
                Err(error) => load.broken.push((name.to_string(), error)),
            }
        }
        load.sites
            .sort_by(|left, right| (left.created_ms, &left.id).cmp(&(right.created_ms, &right.id)));
        load.broken.sort();
        Ok(load)
    }

    fn read_site(&self, path: &Path) -> Result<SiteRecord, String> {
        let text = crate::support_dirs::read_private_text(path, MAX_SITE_BYTES)
            .map_err(|error| error.to_string())?;
        SiteRecord::parse(&text, self.defaults)
    }

    /// The site with this exact ID.
    pub(crate) fn get(&self, id: &str) -> io::Result<Option<SiteRecord>> {
        if !is_site_id(id) {
            return Ok(None);
        }
        let path = self.dir.join(site_file_name(id));
        if !path.exists() {
            return Ok(None);
        }
        self.read_site(&path)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Enters a new site; refused when its ID or its location (C2.1:
    /// „doppelter Ort abgewiesen“) is already entered.
    pub(crate) fn add(&self, site: &SiteRecord) -> Result<(), String> {
        site.validate()?;
        let _guard = self.write_guard()?;
        let load = self.load().map_err(read_error)?;
        let taken_file = site_file_name(&site.id);
        if load.sites.iter().any(|other| other.id == site.id)
            || load.broken.iter().any(|(file, _)| *file == taken_file)
        {
            return Err("Diese Tresorort-ID ist schon vergeben".into());
        }
        check_location_free(&load.sites, site)?;
        self.write(site)
    }

    /// Saves an entered site again (settings, label; unknown keys kept).
    pub(crate) fn save(&self, site: &SiteRecord) -> Result<(), String> {
        site.validate()?;
        let _guard = self.write_guard()?;
        let load = self.load().map_err(read_error)?;
        if !load.sites.iter().any(|other| other.id == site.id) {
            return Err("Tresorort ist nicht eingetragen".into());
        }
        check_location_free(&load.sites, site)?;
        self.write(site)
    }

    /// Removes an entry; `false` when there was none. The vault itself
    /// stays where it is.
    pub(crate) fn remove(&self, id: &str) -> Result<bool, String> {
        if !is_site_id(id) {
            return Err("Tresorort-ID muss aus 32 Hexziffern bestehen".into());
        }
        let _guard = self.write_guard()?;
        match std::fs::remove_file(self.dir.join(site_file_name(id))) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("Tresorliste schreiben: {error}")),
        }
    }

    /// The site `<SITE>` names (C2.2): its ID, a unique label or a unique
    /// ID prefix of at least 4 hex digits.
    pub(crate) fn resolve(&self, query: &str) -> Result<SiteRecord, String> {
        let load = self.load().map_err(read_error)?;
        resolve_in(&load.sites, query)
    }

    fn write(&self, site: &SiteRecord) -> Result<(), String> {
        let path = self.dir.join(site_file_name(&site.id));
        crate::support_dirs::write_private_atomic(&path, site.serialize().as_bytes())
            .map_err(|error| format!("Tresorliste schreiben: {error}"))
    }

    fn write_guard(&self) -> Result<WriteGuard, String> {
        let process = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let locked = (|| {
            crate::support_dirs::ensure_private_dir(&self.dir)?;
            let file = crate::support_dirs::open_private_lock(&self.dir.join(LOCK_FILE))?;
            file.lock()?;
            Ok::<_, io::Error>(file)
        })();
        match locked {
            Ok(file) => Ok(WriteGuard {
                _file: file,
                _process: process,
            }),
            Err(error) => Err(format!("Tresorliste sperren: {error}")),
        }
    }
}

fn read_error(error: io::Error) -> String {
    format!("Tresorliste lesen: {error}")
}

/// A location is entered once (compared as `location_key`).
fn check_location_free(sites: &[SiteRecord], site: &SiteRecord) -> Result<(), String> {
    let key = location_key(&site.location);
    match sites
        .iter()
        .find(|other| other.id != site.id && location_key(&other.location) == key)
    {
        Some(other) => Err(format!(
            "Dieser Speicherort ist schon als Tresorort „{}“ eingetragen",
            other.display_label()
        )),
        None => Ok(()),
    }
}

/// `resolve` over loaded sites: an exact ID first; otherwise every site
/// whose label is `query` or whose ID starts with it (4 hex digits or
/// more) must be the same one.
pub(crate) fn resolve_in(sites: &[SiteRecord], query: &str) -> Result<SiteRecord, String> {
    if query.is_empty() {
        return Err("Kein Tresorort angegeben".into());
    }
    let lower = query.to_ascii_lowercase();
    if is_site_id(&lower) {
        return sites
            .iter()
            .find(|site| site.id == lower)
            .cloned()
            .ok_or_else(|| format!("Kein Tresorort „{query}“"));
    }
    let prefix = is_id_prefix(query);
    let mut matches: Vec<&SiteRecord> = Vec::new();
    for site in sites {
        let hit = site.label == query || (prefix && site.id.starts_with(&lower));
        if hit && !matches.iter().any(|known| known.id == site.id) {
            matches.push(site);
        }
    }
    match matches.as_slice() {
        [site] => Ok((*site).clone()),
        [] => Err(format!("Kein Tresorort „{query}“")),
        _ => Err(format!(
            "„{query}“ passt auf mehrere Tresororte; bitte die ID angeben"
        )),
    }
}
