//! The registry of managed drives under `<AD>/mounts`.
//!
//! DRV-M3 (strand S2 C4 of plan 2026-10-17): `registry.json` keeps the form
//! of v0.5.182 (`{version: 1, mounts: [MountConfig]}` with saved remotes,
//! Google Drive and Share peers), so a rolled-back version keeps reading it.
//! Kinds added since (`Devices`, `Vault`) live only in `registry-2.json`
//! (`mount_registry_ext.rs`) under the same file and size rules. The
//! optional fields of `MountConfig` are written only when set, so an
//! untouched entry keeps its bytes. A change rewrites only the file that
//! holds the drive and then brings the login entry of the drive restore up
//! to date (`drive_autostart.rs`).
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::mount::{MountConfig, MountId};

#[path = "mount_registry_ext.rs"]
mod ext;

const REGISTRY_FILE: &str = "registry.json";
const REGISTRY_VERSION: u32 = 1;
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
/// Drives of both files together: the bound of the worker's drive table.
const MAX_MOUNTS: usize = 256;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Deserialize)]
struct Registry {
    version: u32,
    mounts: Vec<MountConfig>,
}

/// The serialized form of `Registry`, written without cloning the drives.
#[derive(Serialize)]
struct RegistryOut<'a> {
    version: u32,
    mounts: &'a [MountConfig],
}

/// Both files as read for one change.
struct Stored {
    first: Vec<MountConfig>,
    second: ext::SecondFile,
    /// `registry.json` held a later kind (hand edit); the next change moves
    /// it into `registry-2.json`.
    misfiled: bool,
}

pub(super) fn load() -> io::Result<Vec<MountConfig>> {
    load_in(&ensure_registry_directory()?)
}

pub(super) fn upsert(config: &MountConfig) -> io::Result<()> {
    let configs = upsert_in(&ensure_registry_directory()?, config)?;
    super::drive_autostart::reconcile(&configs);
    Ok(())
}

pub(super) fn remove(id: &MountId) -> io::Result<()> {
    let configs = remove_in(&ensure_registry_directory()?, id)?;
    super::drive_autostart::reconcile(&configs);
    Ok(())
}

/// Registered drives for the completion of `se drive` selectors. Never
/// creates the registry directory; any read error yields no candidate.
pub(crate) fn registered_for_completion() -> Vec<MountConfig> {
    let directory = registry_directory();
    if !directory.is_dir() {
        return Vec::new();
    }
    load_in(&directory).unwrap_or_default()
}

pub(super) fn load_in(directory: &Path) -> io::Result<Vec<MountConfig>> {
    Ok(Stored::read(directory)?.configs())
}

/// Stores `config` in the file of its kind and returns every drive.
pub(super) fn upsert_in(directory: &Path, config: &MountConfig) -> io::Result<Vec<MountConfig>> {
    config.validate()?;
    let mut stored = Stored::read(directory)?;
    let first = config.source.stored_in_first_registry();
    let was_first = take(&mut stored.first, &config.id);
    let was_second = take(&mut stored.second.mounts, &config.id);
    if !was_first && !was_second && stored.len() >= MAX_MOUNTS {
        return Err(invalid_data("mount registry is full"));
    }
    if first {
        stored.first.push(config.clone());
    } else {
        stored.second.mounts.push(config.clone());
    }
    stored.write(directory, first || was_first, !first || was_second)?;
    Ok(stored.configs())
}

/// Removes `id` from the file that holds it and returns every other drive.
pub(super) fn remove_in(directory: &Path, id: &MountId) -> io::Result<Vec<MountConfig>> {
    let mut stored = Stored::read(directory)?;
    let was_first = take(&mut stored.first, id);
    let was_second = take(&mut stored.second.mounts, id);
    if was_first || was_second || stored.misfiled {
        stored.write(directory, was_first, was_second)?;
    }
    Ok(stored.configs())
}

impl Stored {
    fn read(directory: &Path) -> io::Result<Self> {
        let first = match read_bounded(&directory.join(REGISTRY_FILE))? {
            Some(body) => parse_first(&body)?,
            None => Vec::new(),
        };
        let mut second = ext::read(directory)?;
        let (first, later): (Vec<_>, Vec<_>) = first
            .into_iter()
            .partition(|config| config.source.stored_in_first_registry());
        let misfiled = !later.is_empty();
        second.mounts.extend(later);
        let stored = Self {
            first,
            second,
            misfiled,
        };
        let mut ids = HashSet::new();
        for config in stored.first.iter().chain(&stored.second.mounts) {
            if !ids.insert(config.id.as_str()) {
                return Err(invalid_data("mount registry contains duplicate ids"));
            }
        }
        if stored.len() > MAX_MOUNTS {
            return Err(invalid_data("mount registry contains too many entries"));
        }
        Ok(stored)
    }

    fn len(&self) -> usize {
        self.first.len() + self.second.len()
    }

    fn configs(&self) -> Vec<MountConfig> {
        let mut configs: Vec<_> = self
            .first
            .iter()
            .chain(&self.second.mounts)
            .cloned()
            .collect();
        configs.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        configs
    }

    /// `registry-2.json` goes first, so a drive that moves between the files
    /// is never missing from both.
    fn write(&mut self, directory: &Path, first: bool, second: bool) -> io::Result<()> {
        if second || self.misfiled {
            ext::write(directory, &mut self.second)?;
        }
        if first || self.misfiled {
            write_first(directory, &mut self.first)?;
        }
        self.misfiled = false;
        Ok(())
    }
}

fn take(mounts: &mut Vec<MountConfig>, id: &MountId) -> bool {
    let before = mounts.len();
    mounts.retain(|config| config.id != *id);
    mounts.len() != before
}

fn parse_first(body: &[u8]) -> io::Result<Vec<MountConfig>> {
    let registry: Registry = serde_json::from_slice(body)
        .map_err(|error| invalid_data(format!("invalid mount registry: {error}")))?;
    if registry.version != REGISTRY_VERSION {
        return Err(invalid_data("unsupported mount registry version"));
    }
    if registry.mounts.len() > MAX_MOUNTS {
        return Err(invalid_data("mount registry contains too many entries"));
    }
    for config in &registry.mounts {
        config.validate()?;
    }
    Ok(registry.mounts)
}

fn write_first(directory: &Path, mounts: &mut [MountConfig]) -> io::Result<()> {
    mounts.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    let registry = RegistryOut {
        version: REGISTRY_VERSION,
        mounts,
    };
    let body = serde_json::to_vec(&registry)
        .map_err(|error| invalid_data(format!("encode mount registry: {error}")))?;
    write_file(directory, REGISTRY_FILE, &body)
}

/// A plain registry file of at most 1 MiB; `None` when it does not exist.
fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    require_plain_file(&metadata)?;
    if metadata.len() > MAX_REGISTRY_BYTES {
        return Err(invalid_data("mount registry exceeds its size limit"));
    }
    let mut body = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(MAX_REGISTRY_BYTES + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > MAX_REGISTRY_BYTES {
        return Err(invalid_data("mount registry exceeds its size limit"));
    }
    Ok(Some(body))
}

/// Replaces `file_name` atomically through a synced temporary file.
fn write_file(directory: &Path, file_name: &str, body: &[u8]) -> io::Result<()> {
    if body.len() as u64 > MAX_REGISTRY_BYTES {
        return Err(invalid_data("mount registry exceeds its size limit"));
    }
    let destination = directory.join(file_name);
    reject_non_file_destination(&destination)?;
    let stem = file_name.trim_end_matches(".json");
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".{stem}.{}.{}.tmp", std::process::id(), sequence));
        let mut file = match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            require_plain_file(&file.metadata()?)?;
            file.write_all(body)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            reject_non_file_destination(&destination)?;
            super::platform::atomic_replace(&temporary, &destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "mount registry temporary filename collision",
    ))
}

fn registry_directory() -> PathBuf {
    crate::support_dirs::app_data_dir().join("mounts")
}

fn ensure_registry_directory() -> io::Result<PathBuf> {
    let directory = registry_directory();
    if !directory.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mount registry path is not absolute",
        ));
    }
    reject_link_ancestors(&directory)?;
    fs::create_dir_all(&directory)?;
    reject_link_ancestors(&directory)?;
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || super::platform::metadata_is_link_like(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "mount registry directory is link-like or not a directory",
        ));
    }
    Ok(directory)
}

fn reject_link_ancestors(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if super::platform::metadata_is_link_like(&metadata) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "mount registry path crosses a link-like ancestor",
                ));
            }
            Ok(metadata) if ancestor != path && !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "mount registry ancestor is not a directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn reject_non_file_destination(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => require_plain_file(&metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn require_plain_file(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.is_file() && !super::platform::metadata_is_link_like(metadata) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "mount registry object is link-like or not a regular file",
        ))
    }
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "registry_compat_tests.rs"]
mod compat_tests;
