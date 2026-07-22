use std::io;
use std::sync::{Arc, Mutex};

use super::core::eio;
use super::fs::{self, ResolvedTarget, ShareExportConfig};
use super::mount_lease::PeerMountLease;
use super::wire::FsMeta;

/// Filesystem routing selected after stream authorization. Stateless browsing
/// resolves the current export table per request; a mounted stream resolves
/// only through its connection-bound lease and the backend retained by it.
#[derive(Clone)]
pub(super) enum FsAccess {
    Dynamic(Arc<Mutex<ShareExportConfig>>),
    Mounted(Arc<PeerMountLease>),
}

impl From<Arc<Mutex<ShareExportConfig>>> for FsAccess {
    fn from(exports: Arc<Mutex<ShareExportConfig>>) -> Self {
        Self::Dynamic(exports)
    }
}

impl FsAccess {
    pub(super) fn dynamic(exports: ShareExportConfig) -> Self {
        Self::Dynamic(Arc::new(Mutex::new(exports)))
    }

    pub(super) fn mounted(lease: Arc<PeerMountLease>) -> Self {
        Self::Mounted(lease)
    }

    pub(super) fn resolve(&self, path: &str) -> io::Result<ResolvedTarget> {
        match self {
            Self::Dynamic(exports) => fs::resolve(path, exports),
            Self::Mounted(lease) => lease.resolve(path),
        }
    }

    pub(super) fn list_dir(&self, path: &str) -> io::Result<Vec<FsMeta>> {
        match self {
            Self::Dynamic(exports) => fs::list_dir(path, exports),
            Self::Mounted(_) => {
                let target = self.resolve(path)?;
                Ok(target
                    .backend
                    .list_dir(&target.path)?
                    .into_iter()
                    .map(Into::into)
                    .collect())
            }
        }
    }

    pub(super) fn stat(&self, path: &str) -> io::Result<FsMeta> {
        match self {
            Self::Dynamic(exports) => fs::stat(path, exports),
            Self::Mounted(_) => {
                let target = self.resolve(path)?;
                Ok(target.backend.stat(&target.path)?.into())
            }
        }
    }

    pub(super) fn rename(
        &self,
        source: &str,
        destination: &str,
        no_replace: bool,
    ) -> io::Result<()> {
        if let Self::Dynamic(exports) = self {
            return fs::rename(source, destination, exports, no_replace);
        }
        let source = self.resolve(source)?;
        let destination = self.resolve(destination)?;
        self.require_same_backend(&source, &destination)?;
        if no_replace {
            source
                .backend
                .rename_no_replace(&source.path, &destination.path)
        } else {
            source.backend.rename(&source.path, &destination.path)
        }
    }

    pub(super) fn promote_staged(&self, staged: &str, destination: &str) -> io::Result<()> {
        if let Self::Dynamic(exports) = self {
            return fs::promote_staged(staged, destination, exports);
        }
        let staged = self.resolve(staged)?;
        let destination = self.resolve(destination)?;
        self.require_same_backend(&staged, &destination)?;
        staged
            .backend
            .promote_staged(&staged.path, &destination.path)
    }

    pub(super) fn require_same_backend(
        &self,
        source: &ResolvedTarget,
        destination: &ResolvedTarget,
    ) -> io::Result<()> {
        let same_generation =
            !matches!(self, Self::Mounted(_)) || Arc::ptr_eq(&source.backend, &destination.backend);
        if source.mount_key == destination.mount_key && same_generation {
            Ok(())
        } else {
            Err(eio("Quelle und Ziel liegen nicht auf derselben Freigabe"))
        }
    }
}
