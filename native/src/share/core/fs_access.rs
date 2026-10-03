use std::io;
use std::sync::{Arc, Mutex};

use super::core::eio;
use super::fs::{self, ResolvedTarget, ShareExportConfig};
use super::mount_lease::PeerMountLease;
use super::wire::FsMeta;
#[path = "fs_authority.rs"]
mod live;
pub(in crate::share) use live::AccessAuthority;
pub(in crate::share) use live::StreamGuard;
#[path = "fs_reversible_replace.rs"]
pub(super) mod reversible_replace;

/// Filesystem routing selected after stream authorization. Stateless browsing
/// resolves the current export table per request; a mounted stream resolves
/// only through its principal/root/policy-bound lease and retained backend.
/// Physical QUIC generations may change without changing that authority.
#[derive(Clone)]
pub(super) enum FsAccess {
    Dynamic(Arc<Mutex<ShareExportConfig>>),
    Mounted(Arc<PeerMountLease>),
    Authorized { access: Box<FsAccess>, authority: Arc<AccessAuthority> },
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

    pub(in crate::share) fn authorized(self, session: Arc<super::session::IncomingSession>,
        auth: Arc<Mutex<super::types::ShareAuthState>>, node: &Arc<super::node::ShareIrohNode>) -> io::Result<Self> {
        let (authority, rights) = AccessAuthority::new(session, auth, node)?;
        let access = match self { Self::Dynamic(_) => Self::dynamic(rights.exports), other => other };
        access.check_read()?;
        Ok(Self::Authorized { access: Box::new(access), authority: Arc::new(authority) })
    }

    pub(super) fn stream_guard(&self) -> Option<StreamGuard> {
        match self { Self::Authorized { authority, .. } => authority.stream_guard(), _ => None }
    }

    pub(super) fn export_snapshot(&self) -> io::Result<ShareExportConfig> {
        self.check_read()?;
        match self {
            Self::Dynamic(exports) => Ok(exports.lock().map_err(|_| eio("Share-Exporte gesperrt"))?.clone()),
            Self::Authorized { access, .. } => access.export_snapshot(),
            Self::Mounted(_) => Err(eio("Exporttabelle ist an eine Mount-Lease gebunden")),
        }
    }

    pub(in crate::share) fn is_dynamic(&self) -> bool {
        match self { Self::Dynamic(_) => true, Self::Mounted(_) => false,
            Self::Authorized { access, .. } => access.is_dynamic() }
    }

    pub(in crate::share) fn check_read(&self) -> io::Result<()> {
        match self { Self::Dynamic(_) => Ok(()), Self::Mounted(lease) => lease.check_live(),
            Self::Authorized { access, authority } => { authority.check()?; access.check_read() } }
    }

    pub(in crate::share) fn check_write(&self) -> io::Result<()> {
        self.check_read()?;
        if let Self::Authorized { authority, .. } = self { authority.check_write()?; }
        Ok(())
    }

    pub(in crate::share) fn register_cancel(&self, cancel: &Arc<std::sync::atomic::AtomicBool>) -> io::Result<()> {
        match self { Self::Authorized { authority, .. } => authority.register_cancel(cancel),
            _ => self.check_read() }
    }

    pub(in crate::share) fn policy_key(&self) -> io::Result<String> {
        self.check_read()?;
        match self {
            Self::Dynamic(exports) => serde_json::to_string(&*exports.lock()
                .map_err(|_| eio("Share-Exporte gesperrt"))?).map_err(io::Error::other),
            Self::Mounted(lease) => Ok(format!("lease:{:p}", Arc::as_ptr(lease))),
            Self::Authorized { access, authority } => Ok(format!("{}:rights:{}",
                access.policy_key()?, authority.revision())),
        }
    }

    pub(in crate::share) fn retained_snapshot(&self) -> io::Result<Self> {
        self.check_read()?;
        Ok(match self {
            Self::Dynamic(exports) => Self::dynamic(exports.lock().map_err(|_| eio("Share-Exporte gesperrt"))?.clone()),
            Self::Mounted(lease) => Self::Mounted(lease.clone()),
            Self::Authorized { access, authority } => Self::Authorized {
                access: Box::new(access.retained_snapshot()?), authority: Arc::new(authority.retained()),
            },
        })
    }

    pub(super) fn resolve_write(&self, path: &str) -> io::Result<ResolvedTarget> {
        self.check_write()?;
        let target = self.resolve(path)?;
        fs::require_target_write(&target)?;
        Ok(target)
    }

    pub(super) fn resolve(&self, path: &str) -> io::Result<ResolvedTarget> {
        match self {
            Self::Dynamic(exports) => fs::resolve(path, exports),
            Self::Mounted(lease) => lease.resolve(path),
            Self::Authorized { access, authority } => {
                authority.check()?;
                super::fs::guard_target(access.resolve(path)?, Some(authority.clone()))
            }
        }
    }

    pub(super) fn list_dir(&self, path: &str) -> io::Result<Vec<FsMeta>> {
        match self {
            Self::Dynamic(exports) => fs::list_dir(path, exports),
            Self::Authorized { access, authority } => {
                authority.check()?;
                let entries = access.list_dir(path)?;
                authority.check()?;
                Ok(entries)
            }
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
            Self::Authorized { access, authority } => {
                authority.check()?;
                let meta = access.stat(path)?;
                authority.check()?;
                Ok(meta)
            }
            Self::Mounted(_) => {
                let target = self.resolve(path)?;
                Ok(target.backend.stat(&target.path)?.into())
            }
        }
    }

    pub(super) fn copy_file(&self, source: &str, destination: &str) -> io::Result<u64> {
        self.check_write()?;
        // Resolve both through this already-authorized export snapshot/lease.
        // Keep the resulting backends (including network handles) alive until
        // the destination's complete stage has been promoted.
        let source = self.resolve(source)?;
        let destination = self.resolve_write(destination)?;
        if source.mount_key == destination.mount_key {
            self.require_same_backend(&source, &destination)?;
            source.backend.copy_file(&source.path, &destination.path)
        } else {
            super::fs_copy::copy_between(
                &*source.backend,
                &source.path,
                &*destination.backend,
                &destination.path,
            )
        }
    }

    pub(super) fn rename(
        &self,
        source: &str,
        destination: &str,
        no_replace: bool,
    ) -> io::Result<()> {
        let source = self.resolve_write(source)?;
        let destination = self.resolve_write(destination)?;
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
        let staged = self.resolve_write(staged)?;
        let destination = self.resolve_write(destination)?;
        self.require_same_backend(&staged, &destination)?;
        staged
            .backend
            .promote_staged(&staged.path, &destination.path)
    }

    /// Publishes a complete stage only while `destination` is absent; the
    /// host validates the stage itself, saving the client a round trip.
    /// `copy` commits a copy stage (ID providers verify their own naming).
    pub(super) fn promote_no_replace(
        &self,
        staged: &str,
        destination: &str,
        copy: bool,
    ) -> io::Result<()> {
        let staged = self.resolve_write(staged)?;
        let destination = self.resolve_write(destination)?;
        self.require_same_backend(&staged, &destination)?;
        if copy {
            staged
                .backend
                .promote_copy_stage(&staged.path, &destination.path)
        } else {
            staged
                .backend
                .promote_staged_no_replace(&staged.path, &destination.path)
        }
    }

    pub(super) fn require_same_backend(
        &self,
        source: &ResolvedTarget,
        destination: &ResolvedTarget,
    ) -> io::Result<()> {
        // Mounted targets always retain this lease's exact backend; wrapping
        // its per-request authority may produce distinct Arc wrapper objects.
        if source.mount_key == destination.mount_key {
            Ok(())
        } else {
            Err(eio("Quelle und Ziel liegen nicht auf derselben Freigabe"))
        }
    }
}
