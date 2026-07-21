use std::{io, path::Path};

use super::{
    engine::MountEngine,
    types::{BackendRoot, MountRuntimeConfig},
};
use crate::vfs::BackendHandle;

impl MountEngine {
    /// Audits only local cache state. Remote access begins in `prepare_host_remote`.
    pub fn open_host_cache(
        config: MountRuntimeConfig,
        backend: BackendHandle,
        spool_root: impl AsRef<Path>,
    ) -> io::Result<Self> {
        Self::open_at_root(
            config,
            BackendRoot::parse("/")?,
            backend,
            spool_root.as_ref(),
            false,
            false,
        )
    }

    pub fn prepare_host_remote(&self) -> io::Result<()> {
        validate_backend_root(&*self.backend, self.projector.root().as_str())?;
        self.recover_pending_deletes()
    }
}

pub(super) fn validate_backend_root(
    backend: &dyn crate::vfs::Backend,
    root: &str,
) -> io::Result<()> {
    let metadata = backend.stat(root)?;
    if metadata.is_dir && !metadata.is_symlink {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "configured mount root is not a plain directory",
    ))
}
