//! Share authorization over host facts and held directory objects.
//!
//! Z-H2: what does not change per request is resolved once. The private
//! roots (app data, cache) and their canonical forms are process constants;
//! a directory whose ancestor walk proved it outside every private root is
//! remembered by its canonical path *and* physical identity for
//! `VERIFIED_FOR`, so the next request into the same folder pays one
//! `stat` instead of an open and a walk to the volume root. The canonical
//! resolution of every target and its containment in the export root still
//! run per request; a directory that was replaced (other identity), moved
//! (other canonical path) or not seen for `VERIFIED_FOR` is walked again.
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use crate::local_access::DirectoryHandle;

use super::{
    export_config::ExportAccess,
    fs_host_destructive, fs_local_paths, fs_path_adapter,
    fs_policy::{normalized, private_path, system_write, within},
};

#[derive(Clone)]
pub(super) struct TargetPolicy {
    pub(super) access: ExportAccess,
    pub(super) allow_system_writes: bool,
    pub(super) local: bool,
    private_roots: Arc<Vec<String>>,
    physical_root: Option<PathBuf>,
}

impl TargetPolicy {
    pub(super) fn new(access: ExportAccess, allow_system_writes: bool, local: bool) -> Self {
        let private_roots = private_root_forms();
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
                if self
                    .physical_root
                    .as_ref()
                    .is_some_and(|root| !fs_path_adapter::policy_contains(root, &effective))
                {
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
                ensure_directory_allowed(directory)?;
            }
        }
        Ok(())
    }

    pub(super) fn destructive(&self, path: &str) -> io::Result<()> {
        self.destructive_observed(path, &|| Ok(()))
    }

    /// `destructive`, reporting every entry the protection walk below a
    /// local folder visits to `visit` (a count, a cancel check); its error
    /// ends the walk. Non-local targets have no walk.
    pub(super) fn destructive_observed(
        &self,
        path: &str,
        visit: &dyn Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        self.write(path)?;
        if self.local {
            let effective = effective(path)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_owned());
            let candidate = normalized(&effective);
            if self
                .private_roots
                .iter()
                .any(|root| within(root, &candidate))
            {
                return Err(denied());
            }
            if Path::new(path).is_dir() {
                fs_host_destructive::check(self, path, visit)?;
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

/// The app's private roots: process constants (the data home and the host
/// configuration are fixed for the life of the process).
fn private_roots() -> &'static [PathBuf] {
    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let mut roots = vec![crate::support_dirs::app_data_dir()];
        if let Some(host) = crate::support_dirs::host() {
            roots.push(host.cache_dir.clone());
        }
        roots
    })
}

/// The normalized spellings of the private roots, as given and canonical.
/// Kept once every root resolved; a root that does not exist yet (no
/// canonical form) is asked again next time, as before.
fn private_root_forms() -> Arc<Vec<String>> {
    static FORMS: OnceLock<Arc<Vec<String>>> = OnceLock::new();
    if let Some(forms) = FORMS.get() {
        return forms.clone();
    }
    let mut complete = true;
    let forms: Vec<String> = private_roots()
        .iter()
        .flat_map(|root| {
            let mut forms = vec![normalized(&root.to_string_lossy())];
            match std::fs::canonicalize(root) {
                Ok(real) => forms.push(normalized(&real.to_string_lossy())),
                Err(_) => complete = false,
            }
            forms
        })
        .collect();
    let forms = Arc::new(forms);
    if complete {
        let _ = FORMS.set(forms.clone());
    }
    forms
}

pub(super) fn ensure_handle_allowed(handle: &DirectoryHandle) -> io::Result<()> {
    if handle.is_within_any(private_roots())? {
        Err(denied())
    } else {
        Ok(())
    }
}

/// How long a directory proven outside every private root stays proven
/// while it keeps its canonical path and identity.
const VERIFIED_FOR: Duration = Duration::from_secs(5);
/// Directories remembered at most; older entries go first.
const VERIFIED_MAX: usize = 4096;

struct Verified {
    identity: fs_path_adapter::DirectoryIdentity,
    at: Instant,
}

fn verified() -> &'static Mutex<HashMap<PathBuf, Verified>> {
    static VERIFIED: OnceLock<Mutex<HashMap<PathBuf, Verified>>> = OnceLock::new();
    VERIFIED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `ensure_handle_allowed` for the existing canonical `directory`. The walk
/// is skipped only when the same path named the same physical directory
/// when it was last proven, less than `VERIFIED_FOR` ago; a directory whose
/// identity cannot be read is always walked.
fn ensure_directory_allowed(directory: &Path) -> io::Result<()> {
    let before = fs_path_adapter::directory_identity(directory);
    if let Some(identity) = before {
        if recently_verified(directory, identity) {
            return Ok(());
        }
    }
    ensure_handle_allowed(&DirectoryHandle::open_root(directory)?)?;
    // Only an object that kept its identity across the walk is remembered.
    if let Some(identity) = before {
        if fs_path_adapter::directory_identity(directory) == Some(identity) {
            remember_verified(directory, identity);
        }
    }
    Ok(())
}

fn recently_verified(directory: &Path, identity: fs_path_adapter::DirectoryIdentity) -> bool {
    // A poisoned table proves nothing: the walk runs.
    verified().lock().ok().is_some_and(|table| {
        table
            .get(directory)
            .is_some_and(|entry| entry.identity == identity && entry.at.elapsed() < VERIFIED_FOR)
    })
}

fn remember_verified(directory: &Path, identity: fs_path_adapter::DirectoryIdentity) {
    let Ok(mut table) = verified().lock() else {
        return;
    };
    if table.len() >= VERIFIED_MAX {
        table.retain(|_, entry| entry.at.elapsed() < VERIFIED_FOR);
        if table.len() >= VERIFIED_MAX {
            table.clear();
        }
    }
    table.insert(
        directory.to_path_buf(),
        Verified {
            identity,
            at: Instant::now(),
        },
    );
}

/// Forgets every proven directory (tests that rearrange folders).
#[cfg(test)]
pub(super) fn forget_verified_directories() {
    if let Ok(mut table) = verified().lock() {
        table.clear();
    }
}

pub(super) fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Pfad ist nicht freigegeben",
    )
}
