use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::mount::{MountConfig, MountId};

const REGISTRY_VERSION: u32 = 1;
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_MOUNTS: usize = 256;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Serialize, Deserialize)]
struct Registry {
    version: u32,
    mounts: Vec<MountConfig>,
}

pub(super) fn load() -> io::Result<Vec<MountConfig>> {
    let directory = ensure_registry_directory()?;
    let path = directory.join("registry.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    require_plain_file(&metadata)?;
    if metadata.len() > MAX_REGISTRY_BYTES {
        return Err(invalid_data("mount registry exceeds its size limit"));
    }
    let mut body = Vec::with_capacity(metadata.len() as usize);
    File::open(&path)?
        .take(MAX_REGISTRY_BYTES + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > MAX_REGISTRY_BYTES {
        return Err(invalid_data("mount registry exceeds its size limit"));
    }
    let registry: Registry = serde_json::from_slice(&body)
        .map_err(|error| invalid_data(format!("invalid mount registry: {error}")))?;
    validate_registry(registry)
}

pub(super) fn upsert(config: &MountConfig) -> io::Result<()> {
    config.validate()?;
    let mut mounts = load()?;
    if let Some(existing) = mounts.iter_mut().find(|existing| existing.id == config.id) {
        *existing = config.clone();
    } else {
        if mounts.len() >= MAX_MOUNTS {
            return Err(invalid_data("mount registry is full"));
        }
        mounts.push(config.clone());
    }
    persist(mounts)
}

pub(super) fn remove(id: &MountId) -> io::Result<()> {
    let mut mounts = load()?;
    let original = mounts.len();
    mounts.retain(|config| config.id != *id);
    if mounts.len() == original {
        return Ok(());
    }
    persist(mounts)
}

fn persist(mut mounts: Vec<MountConfig>) -> io::Result<()> {
    if mounts.len() > MAX_MOUNTS {
        return Err(invalid_data("mount registry is full"));
    }
    mounts.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    let registry = Registry {
        version: REGISTRY_VERSION,
        mounts,
    };
    let body = serde_json::to_vec(&registry)
        .map_err(|error| invalid_data(format!("encode mount registry: {error}")))?;
    if body.len() as u64 > MAX_REGISTRY_BYTES {
        return Err(invalid_data("mount registry exceeds its size limit"));
    }
    let directory = ensure_registry_directory()?;
    let destination = directory.join("registry.json");
    reject_non_file_destination(&destination)?;
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary =
            directory.join(format!(".registry.{}.{}.tmp", std::process::id(), sequence));
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
            file.write_all(&body)?;
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

fn validate_registry(registry: Registry) -> io::Result<Vec<MountConfig>> {
    if registry.version != REGISTRY_VERSION {
        return Err(invalid_data("unsupported mount registry version"));
    }
    if registry.mounts.len() > MAX_MOUNTS {
        return Err(invalid_data("mount registry contains too many entries"));
    }
    let mut ids = HashSet::new();
    for config in &registry.mounts {
        config.validate()?;
        if !ids.insert(config.id.as_str()) {
            return Err(invalid_data("mount registry contains duplicate ids"));
        }
    }
    Ok(registry.mounts)
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
