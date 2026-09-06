use super::case_semantics::identity_key;
use super::metadata_cache::MetadataLookup;
use crate::vfs::VfsMeta;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;
use std::mem::size_of;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[path = "metadata_point_order.rs"]
mod order;
use order::{prune_expired, remove};

const MAX_POINT_BYTES: usize = 16 * 1024 * 1024;
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
    entries: BTreeMap<String, CachedPoint>,
    recency: BTreeSet<(u64, String)>,
    expiry: BTreeSet<(Instant, String)>,
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
        order::touch(&mut state, &key);
        let Some(cached) = state.entries.get(&key) else {
            return Ok((MetadataLookup::Uncached, now));
        };
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
        let key = self.key(path);
        let bytes = key.capacity().saturating_mul(3).saturating_add(192)
            .saturating_add(size_of::<CachedPoint>())
            .saturating_add(metadata.as_ref().map_or(0, meta_bytes));
        if bytes > MAX_POINT_BYTES {
            return Ok(());
        }
        let mut state = self.lock_state()?;
        remove(&mut state, &key);
        let descendants = order::descendants(&state.entries, &key);
        for descendant in descendants {
            remove(&mut state, &descendant);
        }
        prune_expired(&mut state);
        while state.bytes.saturating_add(bytes) > MAX_POINT_BYTES {
            let victim = state.recency.first().map(|(_, key)| key.clone());
            let Some(victim) = victim else {
                return Ok(());
            };
            remove(&mut state, &victim);
        }
        state.clock = state.clock.saturating_add(1);
        let last_touch = state.clock;
        order::insert(&mut state,
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
        let mut removed = if recursive { order::descendants(&state.entries, &key) }
            else { Vec::new() };
        removed.push(key.clone());
        for candidate in removed {
            remove(&mut state, &candidate);
        }
        if let Some((parent, _)) = parent_and_name(path) {
            remove(&mut state, &self.key(parent));
        }
        Ok(())
    }

    pub(super) fn reconcile_directory(&self, path: &str, entries: &[VfsMeta]) -> io::Result<()> {
        self.reconcile_snapshot(path, entries, None)
    }

    pub(super) fn reconcile_snapshot(
        &self, path: &str, entries: &[VfsMeta], previous: Option<&[VfsMeta]>,
    ) -> io::Result<()> {
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
            .map(|metadata| (self.key(&metadata.name), metadata))
            .collect::<BTreeMap<_, _>>();
        let replaced = previous.into_iter().flatten()
            .filter(|old| old.is_dir && !old.is_symlink)
            .filter_map(|old| {
                let name = self.key(&old.name);
                plain_directories.get(&name).is_some_and(|new| old.name != new.name
                    || (old.id.is_some() && new.id.is_some() && old.id != new.id))
                    .then_some(name)
            }).collect::<HashSet<_>>();
        let mut removed = state.entries.range(prefix.clone()..)
            .take_while(|(candidate, _)| candidate.starts_with(&prefix))
            .filter(|(candidate, _)| candidate.as_str() != key)
            .filter(|(candidate, _)| {
                let relative = &candidate[prefix.len()..];
                match relative.split_once('/') {
                    None => true,
                    Some((child, _)) => !plain_directories.contains_key(child)
                        || replaced.contains(child),
                }
            })
            .map(|(candidate, _)| candidate.clone())
            .collect::<Vec<_>>();
        removed.push(key);
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

fn meta_bytes(metadata: &VfsMeta) -> usize {
    size_of::<VfsMeta>()
        .saturating_add(metadata.name.capacity())
        .saturating_add(metadata.id.as_ref().map_or(0, String::capacity))
        .saturating_add(metadata.content_md5.as_ref().map_or(0, String::capacity))
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
