//! Durable memory of nested mounts, shared across processes and successive walks.
use super::version_manifest::{invalid, token};
use crate::vfs::Backend;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Default)]
struct Cache {
    stamp: Option<(SystemTime, u64)>,
    entries: BTreeMap<String, BTreeSet<String>>,
    loaded: bool,
}
static KNOWN: OnceLock<Mutex<Cache>> = OnceLock::new();
fn path() -> std::path::PathBuf {
    crate::support_dirs::sync_data_dir().join("nested-mounts.json")
}
fn stamp() -> io::Result<Option<(SystemTime, u64)>> {
    match std::fs::metadata(path()) {
        Ok(meta) => Ok(Some((meta.modified()?, meta.len()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn load() -> io::Result<BTreeMap<String, BTreeSet<String>>> {
    match crate::support_dirs::read_private_text(
        &path(),
        super::SyncLimits::for_memory(crate::transfer::physical_memory()).state_text_bytes,
    ) {
        Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error),
    }
}
pub(super) fn missing(
    backend: &dyn Backend,
    root: &str,
    path_text: &str,
    mounted: bool,
) -> io::Result<bool> {
    let root = root.trim_end_matches('/');
    let rel = path_text
        .strip_prefix(root)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .ok_or_else(|| invalid("nested mount is outside its sync root"))?;
    if rel.is_empty() {
        return Ok(false);
    }
    let key = token(&format!("{}:{root}", backend.state_identity()));
    let mut cache = KNOWN
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let disk_stamp = stamp()?;
    if !cache.loaded || cache.stamp != disk_stamp {
        cache.entries = load()?;
        cache.stamp = disk_stamp;
        cache.loaded = true;
    }
    if !mounted {
        return Ok(cache
            .entries
            .get(&key)
            .is_some_and(|paths| paths.contains(rel)));
    }
    if cache
        .entries
        .get(&key)
        .is_some_and(|paths| paths.contains(rel))
    {
        return Ok(false);
    }
    // A busy cross-process writer protects this boundary for this walk.
    let _lock = super::pair_lock::PairLock::acquire(&token("nested-mount-memory/v1"))?;
    let mut entries = load()?;
    entries.entry(key).or_default().insert(rel.to_string());
    let text = serde_json::to_vec(&entries).map_err(io::Error::other)?;
    let limit = super::SyncLimits::for_memory(crate::transfer::physical_memory()).state_text_bytes;
    if text.len() as u64 > limit {
        return Err(invalid("nested mount memory exceeds its budget"));
    }
    crate::support_dirs::write_private_atomic(&path(), &text)?;
    cache.entries = entries;
    cache.stamp = stamp()?;
    cache.loaded = true;
    Ok(false)
}
