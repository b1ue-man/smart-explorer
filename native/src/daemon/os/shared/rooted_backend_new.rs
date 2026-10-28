//! Construction of the authorized mount root (`RootedBackend::new_in`):
//! root capabilities, confinement, the root ancestor chain and the first
//! root validation. Moved out of `rooted_backend.rs` unchanged (G4), so the
//! root check cache (ML3) fits the file size rule.
use std::io;
use std::sync::Arc;

use crate::mount::{BackendRoot, MountMode, MountRootSecurity, PathDialect};
use crate::vfs::{BackendHandle, CachingBackend, Scheme, StagedWriteCapabilities};

use super::super::mount_error::encode as sanitize_error;
use super::super::rooted_backend_case::{validate_root, RootCheckCache};
use super::super::rooted_backend_gate::OperationGate;
use super::super::rooted_backend_paths::{
    components, permission_denied, root_ancestor_chain, validate_windows_components,
};
use super::RootedBackend;

impl RootedBackend {
    /// A drive served with the Windows name rules (Dokany).
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub(in super::super) fn new(
        raw_inner: BackendHandle,
        root: &BackendRoot,
        mode: MountMode,
        root_security: MountRootSecurity,
    ) -> io::Result<BackendHandle> {
        Self::new_in(raw_inner, root, mode, root_security, PathDialect::Windows)
    }

    /// A drive served with the name rules of `dialect`: the Windows rules
    /// check every root and path component of a local Windows export; the
    /// POSIX rules (Linux FUSE, DRV-M4) leave names as the backend has them
    /// and never alias Share names.
    pub(in super::super) fn new_in(
        raw_inner: BackendHandle,
        root: &BackendRoot,
        mode: MountMode,
        root_security: MountRootSecurity,
        dialect: PathDialect,
    ) -> io::Result<BackendHandle> {
        let mount_capabilities = raw_inner.mount_path_capabilities(root.as_str())?;
        let root_confinement = mount_capabilities.root_confinement;
        let access = crate::vault::mount_access(&raw_inner, root_confinement.is_enforced())?;
        let vault_trusted = access.is_some() && !root_confinement.is_enforced();
        if root_security == MountRootSecurity::Enforced
            && !root_confinement.is_enforced()
            && !vault_trusted
        {
            return Err(permission_denied(
                "backend does not technically enforce the selected mount root; explicit trusted-root mode is required",
            ));
        }
        // A kernel/provider-confined backend may intentionally be unable to
        // inspect ancestors above its root (Landlock does exactly this). Its
        // capability already covers them; only revalidate the selected root.
        let root_ancestors = if root_confinement.is_enforced() || raw_inner.scheme() == Scheme::Peer
        {
            vec![root.as_str().to_string()]
        } else {
            root_ancestor_chain(root.as_str())?
        };
        let windows_paths = dialect == PathDialect::Windows && raw_inner.is_local();
        if windows_paths {
            validate_windows_components(&components(root.as_str())?)?;
        }
        let staged_write = if mode == MountMode::ReadWrite {
            mount_capabilities.staged_write
        } else {
            StagedWriteCapabilities::default()
        };
        // A local/UNC Windows export and a generic peer can never inherit a
        // case-sensitive claim accidentally. Protocol backends may opt in only
        // after proving the semantics for this exact root.
        let case_sensitive_paths = !windows_paths
            && raw_inner.scheme() != Scheme::Peer
            && raw_inner.case_sensitive_paths(root.as_str());
        let project_peer_names = raw_inner.scheme() == Scheme::Peer;
        let root_metadata = validate_root(&raw_inner, &root_ancestors).map_err(sanitize_error)?;
        if root_metadata.is_symlink || !root_metadata.is_dir {
            return Err(permission_denied(
                "mount root must be an existing plain directory",
            ));
        }
        let revalidate_root =
            root_security == MountRootSecurity::Trusted || !root_confinement.is_enforced();
        let child_key = if case_sensitive_paths {
            None
        } else {
            Some(crate::mount::windows_ordinal_key as fn(&str) -> String)
        };
        let case_cache = Arc::new(CachingBackend::for_mount(Arc::clone(&raw_inner), child_key));
        let inner: BackendHandle = case_cache.clone();
        // G4 ML3: remote roots remember a passed root check until an event
        // (an error, an own folder change, a cache refresh) invalidates it;
        // local roots are checked on every request (a few microseconds).
        let root_check = RootCheckCache::new(!raw_inner.is_local());
        let backend = Self {
            inner,
            case_cache,
            raw_inner,
            root: root.as_str().to_string(),
            root_ancestors,
            revalidate_root,
            root_check,
            read_only: mode == MountMode::ReadOnly,
            windows_paths,
            dialect,
            staged_write,
            case_sensitive_paths,
            project_peer_names,
            root_confinement,
            operation: OperationGate::authorized(access),
        };
        Ok(Arc::new(backend))
    }
}
