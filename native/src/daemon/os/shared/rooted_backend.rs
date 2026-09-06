use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::mount::{BackendRoot, MountMode, MountRootSecurity};
use crate::vfs::{
    Backend, BackendHandle, CachingBackend, DeleteDisposition, MountPathCapabilities,
    RootConfinement, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult,
};

use super::mount_error::{encode as sanitize_error, encoded as coded_error};
use super::rooted_backend_io::{sanitize_metadata, SanitizedReader, SanitizedWriter};
use super::rooted_backend_paths::{
    canonical_virtual_components, components, permission_denied, root_ancestor_chain,
    validate_windows_components,
};

/// A daemon-side authorization boundary for the untrusted mount helper.
/// Every helper path is a canonical virtual `/...` path and is mapped beneath
/// one selected backend root. The original endpoint and account never cross
/// the process boundary.
pub(super) struct RootedBackend {
    // Read resolution may use short-lived metadata, while security-sensitive
    // root and mutation checks always retain the uncached source handle.
    inner: BackendHandle,
    case_cache: Arc<CachingBackend>,
    raw_inner: BackendHandle,
    root: String,
    root_ancestors: Vec<String>,
    revalidate_root: bool,
    read_only: bool,
    windows_paths: bool,
    staged_write: StagedWriteCapabilities,
    case_sensitive_paths: bool,
    root_confinement: RootConfinement,
    operation: super::rooted_backend_gate::OperationGate,
}

impl RootedBackend {
    pub(super) fn new(
        raw_inner: BackendHandle,
        root: &BackendRoot,
        mode: MountMode,
        root_security: MountRootSecurity,
    ) -> io::Result<BackendHandle> {
        let mount_capabilities = raw_inner.mount_path_capabilities(root.as_str())?;
        let root_confinement = mount_capabilities.root_confinement;
        if root_security == MountRootSecurity::Enforced && !root_confinement.is_enforced() {
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
        let windows_paths = raw_inner.is_local();
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
        let root_metadata = super::rooted_backend_case::validate_root(&raw_inner, &root_ancestors)
            .map_err(sanitize_error)?;
        if root_metadata.is_symlink || !root_metadata.is_dir {
            return Err(permission_denied(
                "mount root must be an existing plain directory",
            ));
        }
        let revalidate_root =
            root_security == MountRootSecurity::Trusted || !root_confinement.is_enforced();
        let child_key = if case_sensitive_paths { None } else {
            Some(crate::mount::windows_ordinal_key as fn(&str) -> String)
        };
        let case_cache = Arc::new(CachingBackend::for_mount(Arc::clone(&raw_inner), child_key));
        let inner: BackendHandle = case_cache.clone();
        let backend = Self {
            inner,
            case_cache,
            raw_inner,
            root: root.as_str().to_string(),
            root_ancestors,
            revalidate_root,
            read_only: mode == MountMode::ReadOnly,
            windows_paths,
            staged_write,
            case_sensitive_paths,
            root_confinement,
            operation: super::rooted_backend_gate::OperationGate::new(),
        };
        Ok(Arc::new(backend))
    }

    fn checked_existing(&self, virtual_path: &str) -> io::Result<String> {
        self.checked(virtual_path, false, false)
    }

    fn checked_destination(&self, virtual_path: &str) -> io::Result<String> {
        self.checked(virtual_path, true, true)
    }

    fn checked_existing_for_write(&self, virtual_path: &str) -> io::Result<String> {
        self.checked(virtual_path, false, true)
    }

    fn checked(
        &self,
        virtual_path: &str,
        allow_missing: bool,
        for_write: bool,
    ) -> io::Result<String> {
        (|| {
            let virtual_components = canonical_virtual_components(virtual_path)?;
            if !self.case_sensitive_paths {
                for component in &virtual_components {
                    crate::mount::validate_windows_case_component(component)?;
                }
            }
            if self.windows_paths {
                validate_windows_components(&virtual_components)?;
            }
            let (resolver, case_cache) = if for_write {
                (&self.raw_inner, None)
            } else {
                (&self.inner, Some(self.case_cache.as_ref()))
            };
            super::rooted_backend_case::resolve(
                resolver,
                case_cache,
                self.revalidate_root.then_some(&self.raw_inner),
                &self.root,
                &self.root_ancestors,
                &virtual_components,
                allow_missing,
                self.case_sensitive_paths,
            )
        })()
        .map_err(sanitize_error)
    }

    fn require_write(&self) -> io::Result<()> {
        if self.read_only {
            Err(coded_error(io::ErrorKind::PermissionDenied))
        } else {
            Ok(())
        }
    }

    fn require_child(path: &str) -> io::Result<()> {
        if path == "/" {
            Err(coded_error(io::ErrorKind::PermissionDenied))
        } else {
            Ok(())
        }
    }
}

impl Backend for RootedBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn state_identity(&self) -> String {
        "authorized-mount-root".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let _operation = self.operation.read()?;
        self.inner
            .list_dir(&self.checked_existing(path)?)
            .map(|entries| entries.into_iter().map(sanitize_metadata).collect())
            .map_err(sanitize_error)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let _operation = self.operation.read()?;
        let mut metadata = self
            .raw_inner
            .stat(&self.checked_existing(path)?)
            .map(sanitize_metadata)
            .map_err(sanitize_error)?;
        if path == "/" {
            metadata.name = "/".into();
        }
        Ok(metadata)
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        let _operation = self.operation.read()?;
        let mapped = self.checked_destination(path)?;
        self.inner.try_exists(&mapped).map_err(sanitize_error)
    }

    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        let _operation = self.operation.read()?;
        let _ = self.checked_existing(path)?;
        // Provider object IDs are global capabilities on some backends. Never
        // expose them to, or accept them back from, the mount helper.
        Ok(None)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let _operation = self.operation.read()?;
        let reader = self
            .inner
            .open_read(&self.checked_existing(path)?)
            .map_err(sanitize_error)?;
        Ok(Box::new(SanitizedReader { inner: reader }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(path)?;
        let writer = self
            .inner
            .open_write(&self.checked_destination(path)?)
            .map_err(sanitize_error)?;
        Ok(Box::new(SanitizedWriter { inner: writer }))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(path)?;
        let writer = self
            .inner
            .open_write_new(&self.checked_destination(path)?)
            .map_err(sanitize_error)?;
        Ok(Box::new(SanitizedWriter { inner: writer }))
    }

    fn download_name(&self, path: &str, name: &str) -> String {
        let Ok(_operation) = self.operation.read() else {
            return name.to_string();
        };
        self.checked_existing(path)
            .map(|mapped| self.inner.download_name(&mapped, name))
            .unwrap_or_else(|_| name.to_string())
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(dst)?;
        let src = self.checked_existing_for_write(src)?;
        let dst = self.checked_destination(dst)?;
        self.inner.copy_file(&src, &dst).map_err(sanitize_error)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(src)?;
        Self::require_child(dst)?;
        let src = self.checked_existing_for_write(src)?;
        let dst = self.checked_destination(dst)?;
        self.inner.rename(&src, &dst).map_err(sanitize_error)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(src)?;
        Self::require_child(dst)?;
        let src = self.checked_existing_for_write(src)?;
        let dst = self.checked_destination(dst)?;
        self.inner
            .rename_no_replace(&src, &dst)
            .map_err(sanitize_error)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(staged)?;
        Self::require_child(destination)?;
        let staged = self.checked_existing_for_write(staged)?;
        let destination = self.checked_destination(destination)?;
        self.inner
            .promote_staged(&staged, &destination)
            .map_err(sanitize_error)
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(staged)?;
        Self::require_child(destination)?;
        let staged = self.checked_existing_for_write(staged)?;
        let destination = self.checked_destination(destination)?;
        self.inner
            .promote_staged_no_replace(&staged, &destination)
            .map_err(sanitize_error)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(path)?;
        self.inner
            .remove_file(&self.checked_existing_for_write(path)?)
            .map_err(sanitize_error)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(path)?;
        self.inner
            .remove_dir(&self.checked_existing_for_write(path)?)
            .map_err(sanitize_error)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        self.inner
            .mkdir_all(&self.checked_destination(path)?)
            .map_err(sanitize_error)
    }

    fn delete_disposition(&self) -> DeleteDisposition {
        if self.read_only {
            DeleteDisposition::Unsupported
        } else {
            self.inner.delete_disposition()
        }
    }

    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }

    fn rename_overwrites(&self) -> bool {
        self.staged_write.namespace_replace
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        self.staged_write
    }

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        self.case_sensitive_paths
    }

    fn root_confinement(&self, _root: &str) -> RootConfinement {
        self.root_confinement
    }

    fn mount_path_capabilities(&self, _root: &str) -> VfsResult<MountPathCapabilities> {
        Ok(MountPathCapabilities {
            staged_write: self.staged_write,
            root_confinement: self.root_confinement,
        })
    }

    fn open_read_id(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.open_read(path)
    }

    fn remove_file_id(&self, path: &str, _id: Option<&str>) -> VfsResult<()> {
        self.remove_file(path)
    }

    fn is_local(&self) -> bool {
        // The helper sees a portable virtual `/`, even when the authorized
        // backend happens to be UNC/local. Host-side path normalization must
        // never turn that capability root into a Windows drive path.
        false
    }

    fn provides_content_hash(&self) -> bool {
        self.inner.provides_content_hash()
    }

    fn invalidate_cache(&self) {
        self.inner.invalidate_cache();
    }
}
