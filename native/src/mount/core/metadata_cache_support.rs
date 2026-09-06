use super::{
    identity_key, order, CacheState, CachedDirectory, MetadataLookup, MAX_CACHED_BYTES,
};
use super::load_support::{invalidate_descendants, invalidate_slot, LoadTable};
use crate::vfs::VfsMeta;
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::mem::size_of;
use std::sync::Arc;
use std::time::Instant;

pub(super) fn tick(state: &mut CacheState) -> u64 {
    state.clock = state.clock.saturating_add(1);
    state.clock
}

pub(super) fn lookup_metadata(
    state: &mut CacheState,
    path: &str,
    case_sensitive: bool,
) -> MetadataLookup {
    lookup_metadata_at(state, path, case_sensitive, Instant::now(), true).0
}

pub(super) fn lookup_metadata_at(
    state: &mut CacheState,
    path: &str,
    case_sensitive: bool,
    now: Instant,
    demand: bool,
) -> (MetadataLookup, Instant) {
    let mut cursor = path;
    let mut direct_child = true;
    while let Some((parent, name)) = parent_and_name(cursor) {
        let parent_key = identity_key(case_sensitive, parent);
        let name_key = identity_key(case_sensitive, name);
        if demand { order::touch(state, &parent_key, true); }
        if let Some(cached) = state.directories.get(&parent_key) {
            if cached.listing_expires_at > now {
                let metadata = cached.entry_index.get(&name_key)
                    .and_then(|index| cached.entries.get(*index));
                if direct_child {
                    return (metadata.cloned().map_or(MetadataLookup::KnownMissing,
                        MetadataLookup::Found), cached.listing_expires_at);
                }
                if !metadata.is_some_and(|entry| entry.is_dir && !entry.is_symlink) {
                    return (MetadataLookup::KnownMissing, cached.listing_expires_at);
                }
            }
        }
        cursor = parent;
        direct_child = false;
    }

    let key = identity_key(case_sensitive, path);
    if demand { order::touch(state, &key, true); }
    let Some(cached) = state.directories.get(&key) else {
        return (MetadataLookup::Uncached, now);
    };
    if cached.metadata_expires_at <= now {
        return (MetadataLookup::Uncached, now);
    }
    (MetadataLookup::Found(cached.metadata.clone()), cached.metadata_expires_at)
}

pub(super) fn fits(state: &CacheState, _entries: usize, bytes: usize) -> bool {
    state.bytes.saturating_add(state.cooldown_bytes).saturating_add(bytes) <= MAX_CACHED_BYTES
}

pub(super) fn evict_until(
    state: &mut CacheState,
    needed_entries: usize,
    needed_bytes: usize,
    _root_key: &str,
    _keep_key: Option<&str>,
    _needs_directory_slot: bool,
) {
    while !fits(state, needed_entries, needed_bytes) {
        // Retry hints are less valuable than demanded metadata. Reclaim their
        // byte charge before evicting a usable snapshot for foreground work.
        if let Some((_, key)) = state.cooldown_expiry.first().cloned() {
            order::remove_cooldown(state, &key);
            continue;
        }
        let victim = state.recency.first().map(|(_, key)| key.clone());
        let Some(victim) = victim else {
            break;
        };
        remove_directory(state, &victim);
    }
}

pub(super) fn remove_directory(state: &mut CacheState, key: &str) {
    order::remove(state, key);
}

fn replaced(old: &VfsMeta, new: &VfsMeta) -> bool {
    old.name != new.name || object_replaced(old, new)
}

fn object_replaced(old: &VfsMeta, new: &VfsMeta) -> bool {
    old.is_dir != new.is_dir || old.is_symlink != new.is_symlink
        || (old.id.is_some() && new.id.is_some() && old.id != new.id)
}

pub(super) fn reconcile_loads(
    loads: &mut LoadTable, parent: &str, entries: &[VfsMeta], index: &HashMap<String, usize>,
    previous: Option<(&[VfsMeta], &HashMap<String, usize>)>, case_sensitive: bool,
) {
    let Some((previous, old_index)) = previous else {
        // Initial authority can disprove an already-running descendant fetch.
        invalidate_descendants(loads, parent);
        return;
    };
    for old in previous {
        let key = identity_key(case_sensitive, &old.name);
        let new = index.get(&key).and_then(|index| entries.get(*index));
        if new.is_some_and(|new| super::changes::same(old, new)) { continue; }
        let path = join(parent, &key);
        invalidate_slot(loads, &path);
        if new.map_or(true, |new| replaced(old, new) || !new.is_dir || new.is_symlink) {
            invalidate_descendants(loads, &path);
        }
    }
    for new in entries {
        let key = identity_key(case_sensitive, &new.name);
        if !old_index.contains_key(&key) {
            let path = join(parent, &key);
            invalidate_slot(loads, &path);
            invalidate_descendants(loads, &path);
        }
    }
}

pub(super) fn reconcile_direct_children(
    state: &mut CacheState,
    parent_key: &str,
    entries: &[VfsMeta],
    case_sensitive: bool,
    previous: Option<(&[VfsMeta], &HashMap<String, usize>)>,
) {
    let plain_directories = entries
        .iter()
        .filter(|metadata| metadata.is_dir && !metadata.is_symlink)
        .map(|metadata| (identity_key(case_sensitive, &metadata.name), metadata))
        .collect::<HashMap<_, _>>();
    let mut removed = BTreeSet::new();
    if let Some((old_entries, _)) = previous {
        for old in old_entries.iter().filter(|entry| entry.is_dir && !entry.is_symlink) {
            let name = identity_key(case_sensitive, &old.name);
            if plain_directories.get(&name).map_or(true, |new| replaced(old, new)) {
                removed.insert(join(parent_key, &name));
            }
        }
    } else {
        let prefix = format!("{}/", parent_key.trim_end_matches('/'));
        for (key, cached) in state.directories.range(prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&prefix))
        {
            let relative = &key[prefix.len()..];
            let name = relative.split('/').next().unwrap_or("");
            let changed = plain_directories.get(name).map_or(true, |new| {
                !relative.contains('/') && object_replaced(&cached.metadata, new)
            });
            if changed { removed.insert(join(parent_key, name)); }
        }
    }
    for child in removed {
        let descendants = order::descendants(&state.directories, &child);
        for descendant in descendants { remove_directory(state, &descendant); }
        remove_directory(state, &child);
    }
}

pub(super) fn restore_directory(state: &mut CacheState, key: String, cached: CachedDirectory) {
    order::insert(state, key, cached);
}

pub(super) fn meta_bytes(metadata: &VfsMeta) -> usize {
    size_of::<VfsMeta>()
        .saturating_add(metadata.name.capacity())
        .saturating_add(metadata.id.as_ref().map_or(0, String::capacity))
        .saturating_add(metadata.content_md5.as_ref().map_or(0, String::capacity))
}

pub(super) fn validate_listing(_entries: &[VfsMeta]) -> io::Result<()> {
    // Retention budgets are not filesystem validity limits. The caller still
    // validates names, case collisions and link boundaries before publication.
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
        key_bytes = key_bytes.saturating_add(key.capacity());
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
