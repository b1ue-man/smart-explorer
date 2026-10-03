//! Small atomic sidecars: last seen replicas and completed folder history.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::replica_state::{baseline_file, owner_token, pair_dir};
use super::run_types::{ReplicaRef, StateKey};
use super::snapshot_types::DirSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PairHistory {
    pub replica_a: ReplicaRef,
    pub replica_b: ReplicaRef,
    pub entries_a: u64,
    pub entries_b: u64,
    pub full_ms: i64,
}

impl PairHistory {
    pub fn replica(&self, side: super::PairSide) -> &ReplicaRef {
        match side {
            super::PairSide::A => &self.replica_a,
            super::PairSide::B => &self.replica_b,
        }
    }

    pub fn matches(&self, key: &StateKey) -> bool {
        self.replica_a == key.replica_a && self.replica_b == key.replica_b
    }
}

pub(super) fn history_path(key: &StateKey) -> io::Result<PathBuf> {
    Ok(pair_dir(&key.pair_id).join(format!("{}.replicas.json", owner_token(&key.owner)?)))
}

pub(super) fn load_history(key: &StateKey) -> io::Result<Option<PairHistory>> {
    read_json(&history_path(key)?, 16 * 1024)
}

pub(super) fn save_history(key: &StateKey, history: &PairHistory) -> io::Result<()> {
    write_json(&history_path(key)?, history)
}

pub(super) fn load_dirs(key: &StateKey) -> io::Result<Option<DirSet>> {
    let path = baseline_file(key)?.with_extension("dirs.json");
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let dirs: Option<DirSet> = read_json(&path, limits.state_file_bytes())?;
    if let Some(dirs) = &dirs {
        if dirs.len() as u64 > limits.state_entries
            || dirs
                .iter()
                .fold(0u64, |bytes, rel| bytes.saturating_add(rel.len() as u64))
                > limits.state_text_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "folder history exceeds its budget",
            ));
        }
        for dir in dirs {
            crate::agent_proto::ValidatedRelativePath::parse(dir)?;
        }
    }
    Ok(dirs)
}

pub(super) fn save_dirs(key: &StateKey, dirs: &DirSet) -> io::Result<()> {
    write_json(&baseline_file(key)?.with_extension("dirs.json"), dirs)
}

pub(super) fn index_dirty_path(key: &StateKey) -> io::Result<PathBuf> {
    Ok(baseline_file(key)?.with_extension("index-dirty"))
}

pub(super) fn read_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    limit: u64,
) -> io::Result<Option<T>> {
    let backend = crate::vfs::LocalBackend::new("/");
    let path = path
        .to_str()
        .ok_or_else(|| io::Error::other("state path is not Unicode"))?;
    let metadata = match crate::vfs::Backend::stat(&backend, path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.is_dir || metadata.is_symlink || metadata.special || metadata.size > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sync sidecar exceeds its budget",
        ));
    }
    let file = crate::vfs::open_read_regular(&backend, path, None)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sync sidecar exceeds its budget",
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

pub(super) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    write_bytes(path, &serde_json::to_vec(value).map_err(io::Error::other)?)
}

pub(super) fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("state has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let backend = crate::vfs::LocalBackend::new("/");
    let target = path
        .to_str()
        .ok_or_else(|| io::Error::other("state path is not Unicode"))?;
    let stage = crate::vfs::unique_staging_path(&backend, target, "sync-state")?;
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        crate::vfs::promote_staged_replace(&backend, &stage, target)?;
        crate::vfs::sync_filesystem(&backend, target).map(|_| ())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&stage);
    }
    result
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| i64::try_from(time.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
