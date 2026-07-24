use super::{
    identity_key, CacheState, CachedDirectory, MetadataLookup, MAX_CACHED_BYTES,
    MAX_CACHED_DIRECTORIES, MAX_CACHED_ENTRIES,
};
use crate::vfs::VfsMeta;
use std::collections::{HashMap, HashSet};
use std::io;
use std::mem::size_of;
use std::sync::Arc;

pub(super) fn tick(state: &mut CacheState) -> u64 {
    state.clock = state.clock.saturating_add(1);
    state.clock
}

pub(super) fn lookup_metadata(
    state: &mut CacheState,
    path: &str,
    case_sensitive: bool,
) -> MetadataLookup {
    let touch = tick(state);
    let mut cursor = path;
    let mut direct_child = true;
    while let Some((parent, name)) = parent_and_name(cursor) {
        let parent_key = identity_key(case_sensitive, parent);
        let name_key = identity_key(case_sensitive, name);
        if let Some(cached) = state.directories.get_mut(&parent_key) {
            cached.last_touch = touch;
            cached.last_access = touch;
            let metadata = cached
                .entry_index
                .get(&name_key)
                .and_then(|index| cached.entries.get(*index));
            if direct_child {
                return metadata
                    .cloned()
                    .map_or(MetadataLookup::KnownMissing, MetadataLookup::Found);
            }
            if !metadata.is_some_and(|entry| entry.is_dir && !entry.is_symlink) {
                return MetadataLookup::KnownMissing;
            }
        }
        cursor = parent;
        direct_child = false;
    }

    let key = identity_key(case_sensitive, path);
    let Some(cached) = state.directories.get_mut(&key) else {
        return MetadataLookup::Uncached;
    };
    cached.last_touch = touch;
    cached.last_access = touch;
    MetadataLookup::Found(cached.metadata.clone())
}

pub(super) fn fits(state: &CacheState, entries: usize, bytes: usize) -> bool {
    state.entries.saturating_add(entries) <= MAX_CACHED_ENTRIES
        && state.bytes.saturating_add(bytes) <= MAX_CACHED_BYTES
}

pub(super) fn evict_until(
    state: &mut CacheState,
    needed_entries: usize,
    needed_bytes: usize,
    root_key: &str,
    keep_key: Option<&str>,
    needs_directory_slot: bool,
) {
    while !fits(state, needed_entries, needed_bytes)
        || (needs_directory_slot && state.directories.len() >= MAX_CACHED_DIRECTORIES)
    {
        let victim = state
            .directories
            .iter()
            .filter(|(key, _)| key.as_str() != root_key && keep_key != Some(key.as_str()))
            .min_by_key(|(_, cached)| cached.last_touch)
            .map(|(key, _)| key.clone());
        let Some(victim) = victim else {
            break;
        };
        remove_directory(state, &victim);
    }
}

pub(super) fn remove_directory(state: &mut CacheState, key: &str) {
    if let Some(previous) = state.directories.remove(key) {
        state.entries = state.entries.saturating_sub(previous.entry_count);
        state.bytes = state.bytes.saturating_sub(previous.byte_count);
    }
}

fn remove_subtree(state: &mut CacheState, key: &str) {
    let prefix = format!("{}/", key.trim_end_matches('/'));
    let directories = state
        .directories
        .keys()
        .filter(|candidate| *candidate == key || candidate.starts_with(&prefix))
        .cloned()
        .collect::<Vec<_>>();
    for candidate in directories {
        remove_directory(state, &candidate);
    }
}

pub(super) fn reconcile_direct_children(
    state: &mut CacheState,
    parent_key: &str,
    entries: &[VfsMeta],
    case_sensitive: bool,
) {
    let plain_directories = entries
        .iter()
        .filter(|metadata| metadata.is_dir && !metadata.is_symlink)
        .map(|metadata| identity_key(case_sensitive, &metadata.name))
        .collect::<HashSet<_>>();
    let removed_directories = state
        .directories
        .iter()
        .filter_map(|(key, cached)| {
            let (parent, name) = parent_and_name(&cached.path)?;
            (identity_key(case_sensitive, parent) == parent_key
                && !plain_directories.contains(&identity_key(case_sensitive, name)))
            .then(|| key.clone())
        })
        .collect::<Vec<_>>();
    for child in removed_directories {
        remove_subtree(state, &child);
    }
}

pub(super) fn restore_directory(state: &mut CacheState, key: String, cached: CachedDirectory) {
    state.entries += cached.entry_count;
    state.bytes += cached.byte_count;
    state.directories.insert(key, cached);
}

pub(super) fn meta_bytes(metadata: &VfsMeta) -> usize {
    size_of::<VfsMeta>()
        .saturating_add(metadata.name.len())
        .saturating_add(metadata.id.as_ref().map_or(0, String::len))
        .saturating_add(metadata.content_md5.as_ref().map_or(0, String::len))
}

pub(super) fn validate_listing(entries: &[VfsMeta]) -> io::Result<()> {
    let bytes = entries.iter().fold(0usize, |total, metadata| {
        total.saturating_add(meta_bytes(metadata))
    });
    if entries.len() > MAX_CACHED_ENTRIES || bytes > MAX_CACHED_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "remote directory exceeds the mounted metadata budget",
        ));
    }
    Ok(())
}

pub(super) fn build_entry_index(
    entries: &[VfsMeta],
    case_sensitive: bool,
) -> io::Result<(Arc<HashMap<String, usize>>, usize)> {
    let mut index = HashMap::with_capacity(entries.len());
    let mut key_bytes = 0usize;
    for (entry_index, metadata) in entries.iter().enumerate() {
        let key = identity_key(case_sensitive, &metadata.name);
        key_bytes = key_bytes.saturating_add(key.len());
        if index.insert(key, entry_index).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory contains duplicate names under mount case semantics",
            ));
        }
    }
    let bytes = size_of::<HashMap<String, usize>>()
        .saturating_add(key_bytes)
        .saturating_add(
            index
                .capacity()
                .saturating_mul(size_of::<(String, usize)>().saturating_add(16)),
        );
    Ok((Arc::new(index), bytes))
}

pub(super) fn parent_and_name(path: &str) -> Option<(&str, &str)> {
    if path.is_empty() || path == "/" {
        return None;
    }
    match path.rsplit_once('/') {
        Some(("", name)) if !name.is_empty() => Some(("/", name)),
        Some((parent, name)) if !parent.is_empty() && !name.is_empty() => Some((parent, name)),
        _ => None,
    }
}

pub(super) fn join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}
