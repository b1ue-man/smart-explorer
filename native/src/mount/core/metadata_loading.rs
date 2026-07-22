use super::engine::{lock, read_lock, MountEngine};
use super::path::validate_windows_component;
use crate::vfs::VfsMeta;
use std::collections::HashSet;
use std::io;
use std::sync::{atomic::Ordering, Arc};

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
        stopped: impl Fn() -> bool,
    ) -> io::Result<usize> {
        let targets = self
            .metadata_cache
            .preload_targets(self.config.metadata.preload_depth(), METADATA_PRELOAD_BATCH)?;
        let mut loaded = 0;
        for (path, depth) in targets {
            if stopped() {
                break;
            }
            match self.preload_directory(&path, depth) {
                Ok(true) => loaded += 1,
                Ok(false) => {}
                Err(_) => {
                    let _ = self.metadata_cache.cool_down_snapshot(&path);
                }
            }
        }
        Ok(loaded)
    }

    /// Refreshes a bounded rotating set. A failed or raced refresh leaves the
    /// last complete snapshot intact.
    pub(crate) fn refresh_metadata(&self) -> io::Result<()> {
        self.refresh_metadata_while(|| false)
    }

    pub(crate) fn refresh_metadata_while(&self, stopped: impl Fn() -> bool) -> io::Result<()> {
        let mut first_error = None;
        for (path, depth) in self.metadata_cache.refresh_targets(
            METADATA_REFRESH_BATCH,
            self.config.metadata.preload_depth() > 0,
        )? {
            if stopped() {
                break;
            }
            if let Err(error) = self.refresh_directory(&path, depth) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(super) fn cached_remote_stat(&self, path: &str) -> io::Result<VfsMeta> {
        if let Some(metadata) = self.metadata_cache.stat(path)? {
            return Ok(metadata);
        }
        if let Some(metadata) = self.metadata_points.get(path)? {
            return Ok(metadata);
        }
        let guard = self.metadata_cache.load_guard(path)?;
        let _load = lock(&guard)?;
        if let Some(metadata) = self.metadata_cache.stat(path)? {
            return Ok(metadata);
        }
        if let Some(metadata) = self.metadata_points.get(path)? {
            return Ok(metadata);
        }
        let generation = self.metadata_cache.generation()?;
        let metadata = self.backend.stat(path)?;
        if let Some(snapshot_metadata) = self.metadata_cache.stat(path)? {
            return Ok(snapshot_metadata);
        }
        if self.metadata_cache.generation()? != generation {
            return Ok(metadata);
        }
        self.metadata_points.install(path, metadata.clone())?;
        self.metadata_cache.note_external_observation()?;
        Ok(metadata)
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
        self.metadata_epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.metadata_cache.invalidate(path, recursive);
        let _ = self.metadata_points.invalidate(path, recursive);
    }

    #[cfg(test)]
    pub(super) fn metadata_cache_usage(&self) -> io::Result<(usize, usize, usize)> {
        self.metadata_cache.usage()
    }

    fn load_directory_cached(&self, path: &str, depth: u8) -> io::Result<Arc<[VfsMeta]>> {
        let guard = self.metadata_cache.load_guard(path)?;
        let _load = lock(&guard)?;
        if let Some(entries) = self.metadata_cache.directory(path)? {
            return Ok(entries);
        }
        let generation = self.metadata_cache.generation()?;
        let revision = self.metadata_cache.revision(path)?;
        let (directory, entries) = self.fetch_directory(path)?;
        let entries: Arc<[VfsMeta]> = entries.into();
        let unchanged = self.metadata_cache.generation()? == generation
            && self.metadata_cache.revision(path)? == revision;
        let installed = if unchanged {
            self.install_directory_snapshot(path, directory, Arc::clone(&entries), depth)?
        } else {
            false
        };
        if installed {
            // This path was demanded by a foreground callback, so prioritize
            // it for the next bounded refresh cycle.
            self.metadata_cache.mark_directory_access(path)?;
        } else if unchanged {
            self.metadata_cache.cool_down_snapshot(path)?;
        }
        Ok(entries)
    }

    fn refresh_directory(&self, path: &str, depth: u8) -> io::Result<()> {
        let epoch = {
            let _namespace = read_lock(&self.namespace)?;
            self.metadata_epoch.load(Ordering::Acquire)
        };
        let guard = self.metadata_cache.load_guard(path)?;
        let load = lock(&guard)?;
        let generation = self.metadata_cache.generation()?;
        let revision = self.metadata_cache.revision(path)?;
        let (directory, entries) = self.fetch_directory(path)?;
        let entries: Arc<[VfsMeta]> = entries.into();
        drop(load);
        let _namespace = read_lock(&self.namespace)?;
        let _install_load = lock(&guard)?;
        if self.metadata_epoch.load(Ordering::Acquire) != epoch
            || self.metadata_cache.generation()? != generation
            || self.metadata_cache.revision(path)? != revision
        {
            return Ok(());
        }
        let installed = self.install_directory_snapshot(path, directory, entries, depth)?;
        if !installed {
            self.metadata_cache.cool_down_snapshot(path)?;
        }
        Ok(())
    }

    fn preload_directory(&self, path: &str, depth: u8) -> io::Result<bool> {
        let epoch = {
            let _namespace = read_lock(&self.namespace)?;
            self.metadata_epoch.load(Ordering::Acquire)
        };
        let guard = self.metadata_cache.load_guard(path)?;
        let _load = lock(&guard)?;
        let generation = self.metadata_cache.generation()?;
        let revision = self.metadata_cache.revision(path)?;
        if revision.is_some() {
            return Ok(false);
        }
        let (directory, entries) = self.fetch_directory(path)?;
        let entries: Arc<[VfsMeta]> = entries.into();
        drop(_load);
        let _namespace = read_lock(&self.namespace)?;
        let _install_load = lock(&guard)?;
        if self.metadata_epoch.load(Ordering::Acquire) != epoch
            || self.metadata_cache.generation()? != generation
            || self.metadata_cache.revision(path)? != revision
        {
            return Ok(false);
        }
        let installed = self.install_directory_snapshot(path, directory, entries, depth)?;
        if !installed {
            self.metadata_cache.cool_down_snapshot(path)?;
        }
        Ok(installed)
    }

    fn install_directory_snapshot(
        &self,
        path: &str,
        directory: VfsMeta,
        entries: Arc<[VfsMeta]>,
        depth: u8,
    ) -> io::Result<bool> {
        let installed = self
            .metadata_cache
            .install_directory(path, directory, entries, depth)?;
        if installed {
            self.metadata_points.reconcile_directory(path)?;
        }
        Ok(installed)
    }

    fn fetch_directory(&self, path: &str) -> io::Result<(VfsMeta, Vec<VfsMeta>)> {
        let directory = self.backend.stat(path)?;
        if !directory.is_dir || directory.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mounted directory must be a plain directory",
            ));
        }
        let listed = self.filter_listing(self.backend.list_dir(path)?)?;
        Ok((directory, listed))
    }

    fn filter_listing(&self, mut listed: Vec<VfsMeta>) -> io::Result<Vec<VfsMeta>> {
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
