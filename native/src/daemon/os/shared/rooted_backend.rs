use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::mount::{BackendRoot, MountMode, MountRootSecurity};
use crate::vfs::{
    Backend, BackendHandle, DeleteDisposition, RootConfinement, Scheme, StagedWriteCapabilities,
    VfsMeta, VfsResult,
};

use super::mount_error::{encode as sanitize_error, encoded as coded_error};

/// A daemon-side authorization boundary for the untrusted mount helper.
/// Every helper path is a canonical virtual `/...` path and is mapped beneath
/// one selected backend root. The original endpoint and account never cross
/// the process boundary.
pub(super) struct RootedBackend {
    inner: BackendHandle,
    root: String,
    root_ancestors: Vec<String>,
    read_only: bool,
    windows_paths: bool,
    staged_write: StagedWriteCapabilities,
    case_sensitive_paths: bool,
    root_confinement: RootConfinement,
    operation: Mutex<()>,
}

impl RootedBackend {
    pub(super) fn new(
        inner: BackendHandle,
        root: &BackendRoot,
        mode: MountMode,
        root_security: MountRootSecurity,
    ) -> io::Result<BackendHandle> {
        let root_confinement = inner.root_confinement(root.as_str());
        if root_security == MountRootSecurity::Enforced && !root_confinement.is_enforced() {
            return Err(permission_denied(
                "backend does not technically enforce the selected mount root; explicit trusted-root mode is required",
            ));
        }
        // A kernel/provider-confined backend may intentionally be unable to
        // inspect ancestors above its root (Landlock does exactly this). Its
        // capability already covers them; only revalidate the selected root.
        let root_ancestors = if root_confinement.is_enforced() {
            vec![root.as_str().to_string()]
        } else {
            root_ancestor_chain(root.as_str())?
        };
        let windows_paths = inner.is_local();
        if windows_paths {
            validate_windows_components(&components(root.as_str())?)?;
        }
        let staged_write = if mode == MountMode::ReadWrite {
            inner.staged_write_capabilities(root.as_str())
        } else {
            StagedWriteCapabilities::default()
        };
        // A local/UNC Windows export and a generic peer can never inherit a
        // case-sensitive claim accidentally. Protocol backends may opt in only
        // after proving the semantics for this exact root.
        let case_sensitive_paths = !windows_paths
            && inner.scheme() != Scheme::Peer
            && inner.case_sensitive_paths(root.as_str());
        let backend = Self {
            inner,
            root: root.as_str().to_string(),
            root_ancestors,
            read_only: mode == MountMode::ReadOnly,
            windows_paths,
            staged_write,
            case_sensitive_paths,
            root_confinement,
            operation: Mutex::new(()),
        };
        {
            let _operation = backend.operation_guard()?;
            let mapped = backend.checked_existing("/")?;
            let metadata = backend.inner.stat(&mapped)?;
            if metadata.is_symlink || !metadata.is_dir {
                return Err(permission_denied(
                    "mount root must be an existing plain directory",
                ));
            }
        }
        Ok(Arc::new(backend))
    }

    fn checked_existing(&self, virtual_path: &str) -> io::Result<String> {
        self.checked(virtual_path, false)
    }

    fn checked_destination(&self, virtual_path: &str) -> io::Result<String> {
        self.checked(virtual_path, true)
    }

    fn checked(&self, virtual_path: &str, allow_missing: bool) -> io::Result<String> {
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
            super::rooted_backend_case::resolve(
                &self.inner,
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

    fn operation_guard(&self) -> io::Result<MutexGuard<'_, ()>> {
        self.operation
            .lock()
            .map_err(|_| coded_error(io::ErrorKind::Other))
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
        let _operation = self.operation_guard()?;
        self.inner
            .list_dir(&self.checked_existing(path)?)
            .map(|entries| entries.into_iter().map(sanitize_metadata).collect())
            .map_err(sanitize_error)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let _operation = self.operation_guard()?;
        let mut metadata = self
            .inner
            .stat(&self.checked_existing(path)?)
            .map(sanitize_metadata)
            .map_err(sanitize_error)?;
        if path == "/" {
            metadata.name = "/".into();
        }
        Ok(metadata)
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        let _operation = self.operation_guard()?;
        let mapped = self.checked_destination(path)?;
        self.inner.try_exists(&mapped).map_err(sanitize_error)
    }

    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        let _operation = self.operation_guard()?;
        let _ = self.checked_existing(path)?;
        // Provider object IDs are global capabilities on some backends. Never
        // expose them to, or accept them back from, the mount helper.
        Ok(None)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let _operation = self.operation_guard()?;
        let reader = self
            .inner
            .open_read(&self.checked_existing(path)?)
            .map_err(sanitize_error)?;
        Ok(Box::new(SanitizedReader { inner: reader }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(path)?;
        let writer = self
            .inner
            .open_write(&self.checked_destination(path)?)
            .map_err(sanitize_error)?;
        Ok(Box::new(SanitizedWriter { inner: writer }))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(path)?;
        let writer = self
            .inner
            .open_write_new(&self.checked_destination(path)?)
            .map_err(sanitize_error)?;
        Ok(Box::new(SanitizedWriter { inner: writer }))
    }

    fn download_name(&self, path: &str, name: &str) -> String {
        let Ok(_operation) = self.operation_guard() else {
            return name.to_string();
        };
        self.checked_existing(path)
            .map(|mapped| self.inner.download_name(&mapped, name))
            .unwrap_or_else(|_| name.to_string())
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(dst)?;
        let src = self.checked_existing(src)?;
        let dst = self.checked_destination(dst)?;
        self.inner.copy_file(&src, &dst).map_err(sanitize_error)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(src)?;
        Self::require_child(dst)?;
        let src = self.checked_existing(src)?;
        let dst = self.checked_destination(dst)?;
        self.inner.rename(&src, &dst).map_err(sanitize_error)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(src)?;
        Self::require_child(dst)?;
        let src = self.checked_existing(src)?;
        let dst = self.checked_destination(dst)?;
        self.inner
            .rename_no_replace(&src, &dst)
            .map_err(sanitize_error)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(staged)?;
        Self::require_child(destination)?;
        let staged = self.checked_existing(staged)?;
        let destination = self.checked_destination(destination)?;
        self.inner
            .promote_staged(&staged, &destination)
            .map_err(sanitize_error)
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(staged)?;
        Self::require_child(destination)?;
        let staged = self.checked_existing(staged)?;
        let destination = self.checked_destination(destination)?;
        self.inner
            .promote_staged_no_replace(&staged, &destination)
            .map_err(sanitize_error)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(path)?;
        self.inner
            .remove_file(&self.checked_existing(path)?)
            .map_err(sanitize_error)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let _operation = self.operation_guard()?;
        self.require_write()?;
        Self::require_child(path)?;
        self.inner
            .remove_dir(&self.checked_existing(path)?)
            .map_err(sanitize_error)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let _operation = self.operation_guard()?;
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

fn canonical_virtual_components(path: &str) -> io::Result<Vec<String>> {
    if path == "/" {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        return Err(invalid("mount path must be a canonical absolute path"));
    }
    components(path)
}

fn components(path: &str) -> io::Result<Vec<String>> {
    path.split('/')
        .filter(|component| !component.is_empty())
        .map(|component| {
            if matches!(component, "." | "..")
                || component.contains('\\')
                || component.contains('\0')
            {
                Err(invalid("mount path contains an unsafe component"))
            } else {
                Ok(component.to_string())
            }
        })
        .collect()
}

fn root_ancestor_chain(root: &str) -> io::Result<Vec<String>> {
    let parts = components(root)?;
    if root.starts_with("//") {
        if parts.len() < 2 {
            return Err(invalid("UNC mount root requires server and share"));
        }
        let mut current = format!("//{}/{}", parts[0], parts[1]);
        let mut chain = vec![current.clone()];
        for component in parts.iter().skip(2) {
            current = join(&current, component);
            chain.push(current.clone());
        }
        Ok(chain)
    } else {
        let mut current = "/".to_string();
        let mut chain = vec![current.clone()];
        for component in parts {
            current = join(&current, &component);
            chain.push(current.clone());
        }
        Ok(chain)
    }
}

fn validate_windows_components(components: &[String]) -> io::Result<()> {
    for component in components {
        if (component.ends_with('.') || component.ends_with(' '))
            || component.contains(':')
            || component.chars().any(|character| character < ' ')
        {
            return Err(invalid(
                "mount path is unsafe under Windows path normalization",
            ));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or(component)
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
                });
        if reserved {
            return Err(invalid("mount path uses a reserved Windows device name"));
        }
    }
    Ok(())
}

fn join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn permission_denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn sanitize_metadata(mut metadata: VfsMeta) -> VfsMeta {
    metadata.id = None;
    metadata.content_md5 = metadata.content_md5.and_then(|hash| {
        (hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    });
    metadata
}

struct SanitizedReader {
    inner: Box<dyn Read + Send>,
}

impl Read for SanitizedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer).map_err(sanitize_error)
    }
}

struct SanitizedWriter {
    inner: Box<dyn Write + Send>,
}

impl Write for SanitizedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer).map_err(sanitize_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().map_err(sanitize_error)
    }
}
