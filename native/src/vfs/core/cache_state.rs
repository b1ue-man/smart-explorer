//! State of the directory-listing cache (`CachingBackend`): the cached
//! directories with their recency and expiry order, the live loads, and the
//! constructors of the wrapper.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use super::cache_index;
use super::cache_limits::CacheLimits;
use super::cache_load::{DirectoryLoad, DirectorySnapshot};
use super::{BackendHandle, CachingBackend};

pub(super) struct CachedDirectory {
    pub(super) snapshot: DirectorySnapshot,
    pub(super) entry_count: usize,
    pub(super) byte_count: usize,
    pub(super) last_touch: u64,
}

#[derive(Default)]
pub(super) struct CacheState {
    pub(super) directories: BTreeMap<String, CachedDirectory>,
    pub(super) recency: BTreeSet<(u64, String)>,
    pub(super) expiry: BTreeSet<(Instant, String)>,
    pub(super) loads: BTreeMap<String, Weak<DirectoryLoad>>,
    pub(super) entries: usize,
    pub(super) bytes: usize,
    pub(super) clock: u64,
    // Only explicit whole-cache invalidation advances this epoch. Ordinary
    // mutations fence the affected live DirectoryLoad revisions instead.
    pub(super) generation: u64,
}

impl CachingBackend {
    pub fn new(inner: BackendHandle) -> Self {
        Self::with_child_key(inner, cache_index::exact_child_key)
    }

    pub(crate) fn with_child_key(inner: BackendHandle, child_key: fn(&str) -> String) -> Self {
        Self {
            inner,
            cache: Arc::new(Mutex::new(CacheState::default())),
            child_key,
            limits: CacheLimits::BROWSING,
        }
    }

    pub(crate) fn for_mount(inner: BackendHandle, child_key: Option<fn(&str) -> String>) -> Self {
        let mut cache =
            Self::with_child_key(inner, child_key.unwrap_or(cache_index::exact_child_key));
        cache.limits = CacheLimits::MOUNT;
        cache
    }
}
