//! Same-directory acquisition, separate from disposable retention admission.
use super::{cache_index, cache_support::*, CacheState, CachedDirectory, CachingBackend, CACHE_TTL};
use crate::vfs::{VfsMeta, VfsResult};
use std::{io, sync::{Arc, Mutex, Weak}, time::{Duration, Instant}};

#[derive(Clone)]
pub(super) struct DirectorySnapshot {
    pub entries: Arc<[VfsMeta]>,
    pub index: Arc<cache_index::EntryIndex>,
    pub expires_at: Instant,
}

struct CompletedLoad {
    generation: u64,
    expires_at: Instant,
    result: Result<DirectorySnapshot, SharedFailure>,
}

struct SharedFailure { kind: io::ErrorKind, raw: Option<i32>, message: String }

impl SharedFailure {
    fn capture(error: &io::Error) -> Self {
        Self { kind: error.kind(), raw: error.raw_os_error(), message: error.to_string() }
    }

    fn error(&self) -> io::Error {
        self.raw.map(io::Error::from_raw_os_error)
            .unwrap_or_else(|| io::Error::new(self.kind, self.message.clone()))
    }
}

pub(super) struct DirectoryLoad {
    result: Mutex<Option<CompletedLoad>>,
    cache: Weak<Mutex<CacheState>>,
    key: String,
}

impl Drop for DirectoryLoad {
    fn drop(&mut self) {
        // Remove just this weak registration: no all-table sweep per cold
        // directory, and an unretained snapshot lives only with its waiters.
        if let Some(cache) = self.cache.upgrade() {
            if let Ok(mut cache) = cache.lock() {
                if cache.loads.get(&self.key).is_some_and(|slot| {
                    std::ptr::eq(slot.as_ptr(), self as *const Self)
                }) {
                    cache.loads.remove(&self.key);
                }
            }
        }
    }
}

impl CachingBackend {
    pub(super) fn directory_snapshot(&self, path: &str) -> VfsResult<DirectorySnapshot> {
        self.acquire_directory(path, false)
    }

    /// The mount host already owns its freshness interval. An explicit leaf
    /// refresh must not re-label a daemon-cache hit as a new remote observation.
    /// Shared ancestor resolution still benefits from the resulting snapshot.
    pub(crate) fn refresh_directory(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        Ok(self.acquire_directory(path, true)?.entries.to_vec())
    }

    fn acquire_directory(&self, path: &str, refresh: bool) -> VfsResult<DirectorySnapshot> {
        let key = Self::norm(path);
        let slot = {
            let mut cache = self.cache.lock().map_err(|_| unavailable())?;
            if !refresh {
                if let Some(snapshot) = cached_snapshot(&mut cache, &key) { return Ok(snapshot); }
            }
            if let Some(slot) = cache.loads.get(&key).and_then(Weak::upgrade) {
                slot
            } else {
                let slot = Arc::new(DirectoryLoad {
                    result: Mutex::new(None), cache: Arc::downgrade(&self.cache), key: key.clone(),
                });
                cache.loads.insert(key.clone(), Arc::downgrade(&slot));
                slot
            }
        };
        // Only this path's acquisition lock spans network work. All waiters
        // retain the same slot, including when its result exceeds retention.
        let mut completed = slot.result.lock().map_err(|_| unavailable())?;
        let generation = {
            let mut cache = self.cache.lock().map_err(|_| unavailable())?;
            if !refresh {
                if let Some(snapshot) = cached_snapshot(&mut cache, &key) { return Ok(snapshot); }
            }
            if let Some(result) = completed.as_ref() {
                if result.generation == cache.generation && result.expires_at > Instant::now() {
                    return result.result.as_ref().cloned().map_err(SharedFailure::error);
                }
            }
            cache.generation
        };
        *completed = None;
        let entries: Arc<[VfsMeta]> = match self.inner.list_dir(path) {
            Ok(entries) => entries.into(),
            Err(error) => {
                // An unavailable directory should not turn a simultaneous
                // burst into serialized identical failures either. This is
                // waiter-owned only, never persistent negative authority.
                *completed = Some(CompletedLoad {
                    generation, expires_at: Instant::now() + Duration::from_secs(1),
                    result: Err(SharedFailure::capture(&error)),
                });
                return Err(error);
            }
        };
        let expires_at = Instant::now() + CACHE_TTL;
        // Carry the index with this exact observation. Re-reading the cache
        // could choose another generation, or rebuild a wide unretained index.
        let (index, index_bytes) = cache_index::build(&entries, self.child_key);
        let entry_count = entries.len().saturating_add(1);
        let byte_count = cached_bytes(cached_metadata_bytes(&key, &entries), index_bytes);
        let snapshot = DirectorySnapshot { entries, index: Arc::new(index), expires_at };
        let mut cache = self.cache.lock().map_err(|_| unavailable())?;
        if cache.generation != generation { return Ok(snapshot); }
        *completed = Some(CompletedLoad { generation, expires_at, result: Ok(snapshot.clone()) });
        if entry_count <= self.limits.entries && byte_count <= self.limits.bytes {
            purge_expired(&mut cache);
            remove_directory(&mut cache, &key);
            evict_until(&mut cache, entry_count, byte_count, self.limits);
            if cache.directories.len() < self.limits.directories
                && fits(&cache, entry_count, byte_count, self.limits) {
                let last_touch = tick(&mut cache);
                cache.entries += entry_count;
                cache.bytes += byte_count;
                cache.recency.insert((last_touch, key.clone()));
                cache.expiry.insert((expires_at, key.clone()));
                cache.directories.insert(key, CachedDirectory {
                    snapshot: snapshot.clone(), entry_count, byte_count, last_touch,
                });
            }
        }
        Ok(snapshot)
    }
}

fn unavailable() -> io::Error { io::Error::other("directory cache acquisition state is unavailable") }
