//! Operation-local ownership of removed cache data; never a persistent queue.
use super::{cache_load::DirectoryLoad, CachedDirectory};
use std::sync::Arc;

/// Declare before acquiring cache guards and drain only after releasing them.
/// Retaining upgraded flights also prevents their last Arc from invoking
/// DirectoryLoad::drop (which locks the cache) inside an invalidation lock.
#[derive(Default)]
pub(super) struct Retirement {
    directories: Vec<CachedDirectory>,
    loads: Vec<Arc<DirectoryLoad>>,
}

impl Retirement {
    pub(super) fn directory(&mut self, directory: CachedDirectory) {
        self.directories.push(directory);
    }

    // Called while cache state is locked. No result/acquisition mutex is taken.
    pub(super) fn invalidate_load(&mut self, load: Arc<DirectoryLoad>) {
        load.invalidate();
        self.loads.push(load);
    }

    pub(super) fn clear(&mut self) {
        self.directories.clear();
        self.loads.clear();
    }
}
