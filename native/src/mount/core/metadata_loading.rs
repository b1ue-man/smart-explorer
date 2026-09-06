use super::engine::{not_found, read_lock, MountEngine};
use super::metadata_cache::{Admission, DirectoryObservation,
    MetadataChange, MetadataLookup, DIRECTORY_TTL};
use super::metadata_batch::run_metadata_batch_keyed;
use super::path::validate_windows_component;
use crate::vfs::VfsMeta;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{atomic::Ordering, Arc};
use std::time::Instant;

pub(super) const METADATA_PRELOAD_BATCH: usize = 8;
pub(super) const METADATA_REFRESH_BATCH: usize = 16;

impl MountEngine {
    /// Loads only the complete root snapshot needed for a responsive first
    /// enumeration. Deeper configured levels are filled by the one bounded
    /// background worker after Dokany has made the drive available.
    pub(crate) fn preload_metadata(&self) -> io::Result<()> {
        let depth = self.config.metadata.preload_depth();
        if depth == 0 {
            return Ok(());
        }
        let root = self.projector.root().as_str().to_string();
        self.load_directory_cached(&root, 0).map(|_| ())
    }

    /// Installs one bounded breadth-expansion batch. Failed deeper targets use
    /// a cooldown; on-demand access and local invalidation can recover sooner.
    pub(crate) fn preload_metadata_batch(&self) -> io::Result<usize> {
        self.preload_metadata_batch_while(|| false)
    }

    pub(crate) fn preload_metadata_batch_while(
        &self,
        stopped: impl Fn() -> bool + Sync,
    ) -> io::Result<usize> {
        let targets = self
            .metadata_cache
            .preload_targets(self.config.metadata.preload_depth(), METADATA_PRELOAD_BATCH)?;
        run_metadata_batch_keyed(targets, self.metadata_background_width(), &stopped, &|path, depth| {
            match self.preload_directory(path, depth) {
                Ok(loaded) => Ok(loaded),
                Err(_) => {
                    let _ = self.metadata_cache.cool_down_snapshot(path);
                    Ok(false)
                }
            }
        }, &|path| self.cache_key(path))
    }

    /// Refreshes a bounded rotating set. A failed or raced refresh leaves the
    /// last complete snapshot intact.
    pub(crate) fn refresh_metadata(&self) -> io::Result<()> {
        self.refresh_metadata_while(|| false)
    }

    pub(crate) fn refresh_metadata_while(
        &self, stopped: impl Fn() -> bool + Sync,
    ) -> io::Result<()> {
        let selected = self.metadata_cache.refresh_targets_with_revisions(
            METADATA_REFRESH_BATCH,
            self.config.metadata.preload_depth() > 0,
        )?;
        let revisions = selected.iter().map(|(path, _, revision)|
            (self.cache_key(path), *revision)).collect::<HashMap<_, _>>();
        let targets = selected.into_iter().map(|(path, depth, _)| (path, depth)).collect();
        run_metadata_batch_keyed(targets, self.metadata_background_width(), &stopped, &|path, depth| {
            self.refresh_directory(path, depth, revisions[&self.cache_key(path)])
        }, &|path| self.cache_key(path)).map(|_| ())
    }

    fn metadata_background_width(&self) -> usize {
        self.backend.parallelism().max(1).saturating_sub(1).max(1)
    }

    pub(crate) fn drain_metadata_changes(&self, limit: usize) -> io::Result<Vec<MetadataChange>> {
        self.metadata_cache.drain_changes(limit)
    }

    pub(super) fn cached_remote_stat(&self, path: &str) -> io::Result<VfsMeta> {
        match self.metadata_cache.stat(path)? {
            MetadataLookup::Found(metadata) => return Ok(metadata),
            MetadataLookup::KnownMissing => return Err(not_found(path)),
            MetadataLookup::Uncached => {}
        }
        match self.metadata_points.lookup(path)? {
            MetadataLookup::Found(metadata) => return Ok(metadata),
            MetadataLookup::KnownMissing => return Err(not_found(path)),
            MetadataLookup::Uncached => {}
        }
        if let Some((parent, depth)) = self.metadata_cache.expired_parent(path)? {
            // A known expired parent can answer a whole sibling burst with one
            // shared refresh. Cold/point-fresh stats still avoid parent listing;
            // listing denial never becomes a prerequisite for exact stat.
            let _ = self.load_directory_cached(&parent, depth);
            match self.metadata_cache.stat(path)? {
                MetadataLookup::Found(metadata) => return Ok(metadata),
                MetadataLookup::KnownMissing => return Err(not_found(path)),
                MetadataLookup::Uncached => {}
            }
            match self.metadata_points.lookup(path)? {
                MetadataLookup::Found(metadata) => return Ok(metadata),
                MetadataLookup::KnownMissing => return Err(not_found(path)),
                MetadataLookup::Uncached => {}
            }
        }
        let slot = self.metadata_cache.load_slot(path)?;
        let _load = slot.lock()?;
        match self.metadata_cache.stat(path)? {
            MetadataLookup::Found(metadata) => return Ok(metadata),
            MetadataLookup::KnownMissing => return Err(not_found(path)),
            MetadataLookup::Uncached => {}
        }
        match self.metadata_points.lookup(path)? {
            MetadataLookup::Found(metadata) => return Ok(metadata),
            MetadataLookup::KnownMissing => return Err(not_found(path)),
            MetadataLookup::Uncached => {}
        }
        let revision = slot.revision();
        let fetched = self.backend.stat(path);
        match self.metadata_cache.stat(path)? {
            MetadataLookup::Found(metadata) => return Ok(metadata),
            MetadataLookup::KnownMissing => return Err(not_found(path)),
            MetadataLookup::Uncached => {}
        }
        if slot.revision() != revision {
            return fetched;
        }
        if fetched.as_ref().err().is_some_and(|error| error.kind() != io::ErrorKind::NotFound) {
            return fetched;
        }
        let installed = self.metadata_cache.install_point_if_current(
            path, &slot, revision, &self.metadata_points, fetched.as_ref().ok().cloned(),
        )?;
        match self.metadata_cache.stat(path)? {
            MetadataLookup::Found(snapshot) => {
                let _ = self.metadata_points.invalidate(path, false);
                return Ok(snapshot);
            }
            MetadataLookup::KnownMissing => {
                let _ = self.metadata_points.invalidate(path, false);
                return Err(not_found(path));
            }
            MetadataLookup::Uncached => {}
        }
        let installed_revision = if installed { revision.wrapping_add(1) } else { revision };
        if slot.revision() != installed_revision {
            let _ = self.metadata_points.invalidate(path, false);
            return fetched;
        }
        fetched
    }

    pub(super) fn cached_remote_directory(
        &self,
        path: &str,
        depth: u8,
    ) -> io::Result<Arc<[VfsMeta]>> {
        if let Some(entries) = self.metadata_cache.directory(path)? {
            return Ok(entries);
        }
        self.load_directory_cached(path, depth)
    }

    pub(super) fn invalidate_metadata(&self, path: &str, recursive: bool) {
        self.invalidate_content(path, recursive);
        self.metadata_epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.metadata_cache.invalidate(path, recursive);
        let _ = self.metadata_points.invalidate(path, recursive);
    }

    #[cfg(test)]
    pub(super) fn metadata_cache_usage(&self) -> io::Result<(usize, usize, usize)> {
        self.metadata_cache.usage()
    }

    fn load_directory_cached(&self, path: &str, depth: u8) -> io::Result<Arc<[VfsMeta]>> {
        let slot = self.metadata_cache.load_slot(path)?;
        let _load = slot.lock()?;
        if let Some(entries) = self.metadata_cache.directory(path)? {
            return Ok(entries);
        }
        if let Some(entries) = slot.completed_directory()? {
            return Ok(entries);
        }
        let revision = slot.revision();
        let observation = match self.directory_metadata_hint(path)
            .and_then(|hint| self.fetch_directory(path, hint))
        {
            Ok(observation) => observation,
            Err(error) => {
                slot.complete_directory_failure(revision, &error)?;
                return Err(error);
            }
        };
        let entries = Arc::clone(&observation.entries);
        let expires_at = observation.listing_expires_at;
        let installed = self.install_directory_snapshot(
            path,
            observation,
            depth,
            &slot,
            revision,
            Admission::Demand,
        )?;
        if installed {
            // This path was demanded by a foreground callback, so prioritize
            // it for the next bounded refresh cycle.
            self.metadata_cache.mark_directory_access(path)?;
        } else if slot.revision() == revision && self.metadata_cache.revision(path)?.is_none() {
            self.metadata_cache.cool_down_snapshot(path)?;
        }
        // Successful admission increments this path's revision exactly once.
        // A concurrent invalidation must never tag old entries with its newer
        // revision; publish against the expected value, not a reread value.
        let completed_revision = if installed { revision.wrapping_add(1) } else { revision };
        slot.complete_directory(completed_revision, expires_at, Arc::clone(&entries))?;
        Ok(entries)
    }

    fn refresh_directory(
        &self, path: &str, depth: u8, selected_revision: Option<u64>,
    ) -> io::Result<bool> {
        let epoch = {
            let _namespace = read_lock(&self.namespace)?;
            self.metadata_epoch.load(Ordering::Acquire)
        };
        let slot = self.metadata_cache.load_slot(path)?;
        let load = slot.lock()?;
        // Preserve explicit refresh semantics for unchanged selections; only a
        // newer still-fresh snapshot satisfies work selected before its fetch.
        if self.metadata_cache.refreshed_since(path, selected_revision)? {
            return Ok(false);
        }
        let revision = slot.revision();
        let hint = self.directory_metadata_hint(path)?;
        let observation = self.fetch_directory(path, hint)?;
        drop(load);
        let _namespace = read_lock(&self.namespace)?;
        let _install_load = slot.lock()?;
        if self.metadata_epoch.load(Ordering::Acquire) != epoch || slot.revision() != revision {
            return Ok(false);
        }
        let installed = self.install_directory_snapshot(
            path, observation, depth, &slot, revision, Admission::Refresh,
        )?;
        Ok(installed)
    }

    fn preload_directory(&self, path: &str, depth: u8) -> io::Result<bool> {
        let epoch = {
            let _namespace = read_lock(&self.namespace)?;
            self.metadata_epoch.load(Ordering::Acquire)
        };
        let slot = self.metadata_cache.load_slot(path)?;
        let load = slot.lock()?;
        let revision = slot.revision();
        if self.metadata_cache.revision(path)?.is_some() {
            return Ok(false);
        }
        let hint = self.directory_metadata_hint(path)?;
        let observation = self.fetch_directory(path, hint)?;
        drop(load);
        let _namespace = read_lock(&self.namespace)?;
        let _install_load = slot.lock()?;
        if self.metadata_epoch.load(Ordering::Acquire) != epoch || slot.revision() != revision {
            return Ok(false);
        }
        let installed = self.install_directory_snapshot(
            path, observation, depth, &slot, revision, Admission::Speculative,
        )?;
        if !installed && slot.revision() == revision {
            self.metadata_cache.cool_down_snapshot(path)?;
        }
        Ok(installed)
    }

    fn install_directory_snapshot(
        &self,
        path: &str,
        observation: DirectoryObservation,
        depth: u8,
        slot: &super::metadata_cache::LoadSlot,
        revision: u64,
        intent: Admission,
    ) -> io::Result<bool> {
        self.metadata_cache.install_observation_reconciled(
            path, observation, depth, Some((slot, revision)), intent, &self.metadata_points,
        )
    }

    fn directory_metadata_hint(&self, path: &str) -> io::Result<Option<(VfsMeta, Instant)>> {
        match self.metadata_cache.metadata_hint(path)? {
            Some(hint) => Ok(Some(hint)),
            None => self.metadata_points.metadata_hint(path),
        }
    }

    fn fetch_directory(
        &self,
        path: &str,
        hint: Option<(VfsMeta, Instant)>,
    ) -> io::Result<DirectoryObservation> {
        let (directory, metadata_expires_at) = match hint {
            Some((metadata, expires_at)) if expires_at > Instant::now() => (metadata, expires_at),
            _ => (self.backend.stat(path)?, Instant::now() + DIRECTORY_TTL),
        };
        if !directory.is_dir || directory.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mounted directory must be a plain directory",
            ));
        }
        let listed = self.filter_listing(self.backend.list_dir(path)?)?;
        Ok(DirectoryObservation {
            metadata: directory, metadata_expires_at,
            entries: listed.into(), listing_expires_at: Instant::now() + DIRECTORY_TTL,
        })
    }

    fn filter_listing(&self, mut listed: Vec<VfsMeta>) -> io::Result<Vec<VfsMeta>> {
        self.metadata_cache.validate_listing(&listed)?;
        let mut names = HashSet::new();
        let mut duplicate = false;
        listed.retain(|metadata| {
            if is_reserved_mount_sibling(&metadata.name)
                || validate_windows_component(&metadata.name).is_err()
                || (!self.case_sensitive_paths()
                    && crate::mount::validate_windows_case_component(&metadata.name).is_err())
            {
                return false;
            }
            if !names.insert(self.name_key(&metadata.name)) {
                duplicate = true;
            }
            true
        });
        if duplicate {
            return Err(super::engine::invalid_data(
                "backend contains non-unique child names under mount case semantics",
            ));
        }
        Ok(listed)
    }
}

fn is_reserved_mount_sibling(name: &str) -> bool {
    [".se-mount-", ".se-mount-delete-"]
        .into_iter()
        .any(|marker| {
            name.rsplit_once(marker).is_some_and(|(base, suffix)| {
                !base.is_empty()
                    && suffix.len() == 16
                    && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        })
}
