use super::case_semantics::identity_key;
use super::metadata_cache::MetadataLookup;
use crate::vfs::VfsMeta;
use std::collections::{HashMap, HashSet};
use std::io;
use std::mem::size_of;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_POINT_STATS: usize = 4_096;
const MAX_POINT_BYTES: usize = 4 * 1024 * 1024;
const POINT_STAT_TTL: Duration = Duration::from_secs(5);
const POINT_MISSING_TTL: Duration = Duration::from_secs(1);

struct CachedPoint {
    metadata: Option<VfsMeta>,
    bytes: usize,
    expires_at: Instant,
    last_touch: u64,
}

#[derive(Default)]
struct PointState {
    entries: HashMap<String, CachedPoint>,
    bytes: usize,
    clock: u64,
}

pub(super) struct MetadataPointCache {
    case_sensitive: bool,
    state: Mutex<PointState>,
}

impl MetadataPointCache {
    pub(super) fn new(case_sensitive: bool) -> Self {
        Self {
            case_sensitive,
            state: Mutex::new(PointState::default()),
        }
    }

    pub(super) fn get(&self, path: &str) -> io::Result<Option<VfsMeta>> {
        Ok(match self.lookup(path)? {
            MetadataLookup::Found(metadata) => Some(metadata),
            MetadataLookup::KnownMissing | MetadataLookup::Uncached => None,
        })
    }

    pub(super) fn lookup(&self, path: &str) -> io::Result<MetadataLookup> {
        Ok(self.lookup_at(path)?.0)
    }

    pub(super) fn metadata_hint(&self, path: &str) -> io::Result<Option<(VfsMeta, Instant)>> {
        let (lookup, expires_at) = self.lookup_at(path)?;
        match lookup {
            MetadataLookup::Found(metadata) => Ok(Some((metadata, expires_at))),
            MetadataLookup::KnownMissing => Err(io::Error::new(
                io::ErrorKind::NotFound, "mounted metadata path does not exist",
            )),
            MetadataLookup::Uncached => Ok(None),
        }
    }

    fn lookup_at(&self, path: &str) -> io::Result<(MetadataLookup, Instant)> {
        let key = self.key(path);
        let mut state = self.lock_state()?;
        let now = Instant::now();
        if state
            .entries
            .get(&key)
            .is_some_and(|cached| cached.expires_at <= now)
        {
            remove(&mut state, &key);
            return Ok((MetadataLookup::Uncached, now));
        }
        state.clock = state.clock.saturating_add(1);
        let touch = state.clock;
        let Some(cached) = state.entries.get_mut(&key) else {
            return Ok((MetadataLookup::Uncached, now));
        };
        cached.last_touch = touch;
        Ok((cached.metadata.clone().map_or(MetadataLookup::KnownMissing,
            MetadataLookup::Found), cached.expires_at))
    }

    pub(super) fn install(&self, path: &str, metadata: VfsMeta) -> io::Result<()> {
        self.install_observation(path, Some(metadata))
    }

    pub(super) fn install_missing(&self, path: &str) -> io::Result<()> {
        self.install_observation(path, None)
    }

    fn install_observation(&self, path: &str, metadata: Option<VfsMeta>) -> io::Result<()> {
        let ttl = if metadata.is_some() { POINT_STAT_TTL } else { POINT_MISSING_TTL };
        let bytes = path.len().saturating_add(size_of::<CachedPoint>())
            .saturating_add(metadata.as_ref().map_or(0, meta_bytes));
        if bytes > MAX_POINT_BYTES {
            return Ok(());
        }
        let key = self.key(path);
        let mut state = self.lock_state()?;
        remove(&mut state, &key);
        let prefix = format!("{}/", key.trim_end_matches('/'));
        let descendants = state.entries.keys().filter(|candidate| candidate.starts_with(&prefix))
            .cloned().collect::<Vec<_>>();
        for descendant in descendants {
            remove(&mut state, &descendant);
        }
        prune_expired(&mut state);
        while state.entries.len() >= MAX_POINT_STATS
            || state.bytes.saturating_add(bytes) > MAX_POINT_BYTES
        {
            let victim = state
                .entries
                .iter()
                .min_by_key(|(_, cached)| cached.last_touch)
                .map(|(key, _)| key.clone());
            let Some(victim) = victim else {
                return Ok(());
            };
            remove(&mut state, &victim);
        }
        state.clock = state.clock.saturating_add(1);
        let last_touch = state.clock;
        state.bytes += bytes;
        state.entries.insert(
            key,
            CachedPoint {
                metadata,
                bytes,
                expires_at: Instant::now() + ttl,
                last_touch,
            },
        );
        Ok(())
    }

    pub(super) fn invalidate(&self, path: &str, recursive: bool) -> io::Result<()> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        let prefix = format!("{}/", key.trim_end_matches('/'));
        let removed = state
            .entries
            .keys()
            .filter(|candidate| *candidate == &key || (recursive && candidate.starts_with(&prefix)))
            .cloned()
            .collect::<Vec<_>>();
        for candidate in removed {
            remove(&mut state, &candidate);
        }
        if let Some((parent, _)) = parent_and_name(path) {
            remove(&mut state, &self.key(parent));
        }
        Ok(())
    }

    pub(super) fn reconcile_directory(&self, path: &str, entries: &[VfsMeta]) -> io::Result<()> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        let prefix = if key == "/" {
            "/".to_string()
        } else {
            format!("{key}/")
        };
        let plain_directories = entries
            .iter()
            .filter(|metadata| metadata.is_dir && !metadata.is_symlink)
            .map(|metadata| self.key(&metadata.name))
            .collect::<HashSet<_>>();
        let removed = state
            .entries
            .keys()
            .filter(|candidate| {
                if *candidate == &key {
                    return true;
                }
                let Some(relative) = candidate.strip_prefix(&prefix) else {
                    return false;
                };
                match relative.split_once('/') {
                    None => true,
                    Some((child, _)) => !plain_directories.contains(child),
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        for candidate in removed {
            remove(&mut state, &candidate);
        }
        Ok(())
    }

    fn key(&self, path: &str) -> String {
        identity_key(self.case_sensitive, path)
    }

    fn lock_state(&self) -> io::Result<std::sync::MutexGuard<'_, PointState>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("mount point metadata cache is unavailable"))
    }
}

fn prune_expired(state: &mut PointState) {
    let now = Instant::now();
    let expired = state
        .entries
        .iter()
        .filter(|(_, cached)| cached.expires_at <= now)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in expired {
        remove(state, &key);
    }
}

fn remove(state: &mut PointState, key: &str) {
    if let Some(cached) = state.entries.remove(key) {
        state.bytes = state.bytes.saturating_sub(cached.bytes);
    }
}

fn meta_bytes(metadata: &VfsMeta) -> usize {
    size_of::<VfsMeta>()
        .saturating_add(metadata.name.len())
        .saturating_add(metadata.id.as_ref().map_or(0, String::len))
        .saturating_add(metadata.content_md5.as_ref().map_or(0, String::len))
}

fn parent_and_name(path: &str) -> Option<(&str, &str)> {
    if path.is_empty() || path == "/" {
        return None;
    }
    match path.rsplit_once('/') {
        Some(("", name)) if !name.is_empty() => Some(("/", name)),
        Some((parent, name)) if !parent.is_empty() && !name.is_empty() => Some((parent, name)),
        _ => None,
    }
}
