//! Share authorization over host facts and held directory objects.
use std::{
    io,
    path::{Path, PathBuf},
};

use crate::local_access::DirectoryHandle;

use super::{
    export_config::ExportAccess,
    fs_host_destructive,
    fs_local_paths,
    fs_path_adapter,
    fs_policy::{normalized, private_path, system_write, within},
};

#[derive(Clone)]
pub(super) struct TargetPolicy {
    pub(super) access: ExportAccess,
    pub(super) allow_system_writes: bool,
    pub(super) local: bool,
    private_roots: Vec<String>,
    physical_root: Option<PathBuf>,
}

impl TargetPolicy {
    pub(super) fn new(access: ExportAccess, allow_system_writes: bool, local: bool) -> Self {
        let private_roots = private_roots()
            .into_iter()
            .flat_map(|root| {
                let mut forms = vec![normalized(&root.to_string_lossy())];
                if let Ok(real) = std::fs::canonicalize(root) {
                    forms.push(normalized(&real.to_string_lossy()));
                }
                forms
            })
            .collect();
        Self {
            access,
            allow_system_writes,
            local,
            private_roots,
            physical_root: None,
        }
    }

    pub(super) fn with_root(mut self, root: &str) -> Self {
        if self.local {
            self.physical_root = Some(fs_local_paths::to_os_path(root));
        }
        self
    }

    fn app_private(&self, path: &str) -> bool {
        let path = normalized(path);
        self.private_roots.iter().any(|root| within(&path, root))
    }

    pub(super) fn visible(&self, path: &str, link: bool) -> bool {
        if private_path(path) || (self.local && self.app_private(path)) {
            return false;
        }
        !link || self.read(path).is_ok()
    }

    pub(super) fn read(&self, path: &str) -> io::Result<()> {
        if private_path(path) {
            return Err(denied());
        }
        if self.local {
            if self.app_private(path) {
                return Err(denied());
            }
            if let Some(effective) = effective(path) {
                if self.physical_root.as_ref().is_some_and(|root| {
                    !fs_path_adapter::policy_contains(root, &effective)
                }) {
                    return Err(denied());
                }
                if self.app_private(&effective.to_string_lossy())
                    || private_path(&effective.to_string_lossy())
                {
                    return Err(denied());
                }
                let mut directory = effective.as_path();
                while !directory.is_dir() {
                    directory = directory.parent().ok_or_else(denied)?;
                }
                ensure_handle_allowed(&DirectoryHandle::open_root(directory)?)?;
            }
        }
        Ok(())
    }

    pub(super) fn destructive(&self, path: &str) -> io::Result<()> {
        self.write(path)?;
        if self.local {
            let effective = effective(path)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_owned());
            let candidate = normalized(&effective);
            if self.private_roots.iter().any(|root| within(root, &candidate)) {
                return Err(denied());
            }
            if Path::new(path).is_dir() {
                fs_host_destructive::check(self, path)?;
            }
        }
        Ok(())
    }

    pub(super) fn write(&self, path: &str) -> io::Result<()> {
        self.read(path)?;
        if !self.access.allows_write() {
            return Err(io::Error::new(
                io::ErrorKind::ReadOnlyFilesystem,
                "Freigabe ist nur lesbar",
            ));
        }
        if self.local && !self.allow_system_writes {
            if system_write(path) {
                return Err(denied());
            }
            if let Some(effective) = effective(path) {
                if system_write(&effective.to_string_lossy()) {
                    return Err(denied());
                }
            }
        }
        Ok(())
    }
}

fn effective(path: &str) -> Option<PathBuf> {
    // New stages/files still need the effective target of an existing parent.
    let mut missing = Vec::new();
    let mut parent = Path::new(path);
    loop {
        if let Ok(mut resolved) = std::fs::canonicalize(parent) {
            for name in missing.into_iter().rev() {
                resolved.push(name);
            }
            return Some(resolved);
        }
        missing.push(parent.file_name()?.to_owned());
        parent = parent.parent()?;
    }
}

fn private_roots() -> Vec<PathBuf> {
    let mut roots = vec![crate::support_dirs::app_data_dir()];
    if let Some(host) = crate::support_dirs::host() {
        roots.push(host.cache_dir.clone());
    }
    roots
}

pub(super) fn ensure_handle_allowed(handle: &DirectoryHandle) -> io::Result<()> {
    if handle.is_within_any(&private_roots())? {
        Err(denied())
    } else {
        Ok(())
    }
}

pub(super) fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "Pfad ist nicht freigegeben")
}
