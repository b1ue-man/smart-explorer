//! Share policy over literal names and already resolved host facts.
use std::{io, path::{Path, PathBuf}};
use super::export_config::ExportAccess;
#[path = "fs_policy_destructive.rs"]
mod destructive;

#[derive(Clone)]
pub(super) struct TargetPolicy {
    pub(super) access: ExportAccess,
    pub(super) allow_system_writes: bool,
    pub(super) local: bool,
    private_roots: Vec<String>,
    physical_root: Option<PathBuf>,
}

pub(super) fn private_name(name: &str) -> bool {
    let name = name.split(':').next().unwrap_or(name).trim_end_matches(['.', ' ']).to_ascii_lowercase();
    name == ".se-versions" || name.strip_prefix(".held.se-recycle-")
        .is_some_and(|suffix| suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(super) fn private_path(path: &str) -> bool {
    path.split(['/', '\\']).any(private_name)
}

fn within(path: &str, root: &str) -> bool {
    path == root || path.strip_prefix(root.trim_end_matches('/')).is_some_and(|rest| rest.starts_with('/'))
}

fn normalized(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("//?/").unwrap_or(&path);
    let path = path.strip_prefix("UNC/").map_or_else(|| path.to_owned(), |tail| format!("//{tail}"));
    path.trim_end_matches('/').to_ascii_lowercase()
}

fn effective(path: &str) -> Option<std::path::PathBuf> {
    // New stages/files still need the effective target of an existing parent.
    let mut missing = Vec::new();
    let mut parent = Path::new(path);
    loop {
        if let Ok(mut resolved) = std::fs::canonicalize(parent) {
            for name in missing.into_iter().rev() { resolved.push(name); }
            return Some(resolved);
        }
        missing.push(parent.file_name()?.to_owned());
        parent = parent.parent()?;
    }
}

pub(super) fn system_write(path: &str) -> bool {
    let path = normalized(path);
    let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| matches!(*p, ".ssh" | ".gnupg" | ".aws" | ".kube")) { return true; }
    if parts.last().is_some_and(|p| matches!(*p, ".profile" | ".bashrc" | ".bash_profile" |
        ".bash_login" | ".bash_logout" | ".zshrc" | ".zshenv" | ".zprofile" | ".zlogin" | ".zlogout"
        | ".login" | ".cshrc" | ".pam_environment")) { return true; }
    if ["/etc", "/usr", "/bin", "/sbin", "/boot", "/lib", "/lib64", "/var/lib/systemd"]
        .iter().any(|root| within(&path, root)) { return true; }
    ["/.config/autostart", "/.config/systemd", "/.local/share/systemd", "/.config/fish", "/.config/powershell",
        "/microsoft/windows/start menu/programs/startup", "/microsoft/crypto", "/microsoft/credentials",
        "/documents/windowspowershell", "/documents/powershell"]
        .iter().any(|marker| path.ends_with(marker) || path.contains(&format!("{marker}/")))
        || parts.get(1).is_some_and(|p| parts[0].ends_with(':') && matches!(*p, "windows" | "program files" | "program files (x86)"))
}

impl TargetPolicy {
    pub(super) fn new(access: ExportAccess, allow_system_writes: bool, local: bool) -> Self {
        let roots = private_roots();
        let private_roots = roots.into_iter().flat_map(|root| {
            let mut forms = vec![normalized(&root.to_string_lossy())];
            if let Ok(real) = std::fs::canonicalize(root) { forms.push(normalized(&real.to_string_lossy())); }
            forms
        }).collect();
        Self { access, allow_system_writes, local, private_roots, physical_root: None }
    }
    pub(super) fn with_root(mut self, root: &str) -> Self {
        if self.local { self.physical_root = Some(super::fs::local_paths::to_os_path(root)); }
        self
    }
    fn app_private(&self, path: &str) -> bool {
        let path = normalized(path);
        self.private_roots.iter().any(|root| within(&path, root))
    }
    pub(super) fn visible(&self, path: &str, link: bool) -> bool {
        if private_path(path) || (self.local && self.app_private(path)) { return false; }
        !link || self.read(path).is_ok()
    }

    pub(super) fn read(&self, path: &str) -> io::Result<()> {
        if private_path(path) { return Err(denied()); }
        if self.local {
            if self.app_private(path) { return Err(denied()); }
            if let Some(effective) = effective(path) {
                if self.physical_root.as_ref().is_some_and(|root| {
                    if std::path::MAIN_SEPARATOR == '\\' {
                        !within(&normalized(&effective.to_string_lossy()), &normalized(&root.to_string_lossy()))
                    } else { !effective.starts_with(root) }
                }) { return Err(denied()); }
                if self.app_private(&effective.to_string_lossy()) || private_path(&effective.to_string_lossy()) { return Err(denied()); }
                let mut directory = effective.as_path();
                while !directory.is_dir() {
                    directory = directory.parent().ok_or_else(denied)?;
                }
                ensure_handle_allowed(&crate::local_access::DirectoryHandle::open_root(directory)?)?;
            }
        }
        Ok(())
    }
    pub(super) fn destructive(&self, path: &str) -> io::Result<()> {
        self.write(path)?;
        if self.local {
            let effective = effective(path).map(|path| path.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_owned());
            let candidate = normalized(&effective);
            if self.private_roots.iter().any(|root| within(root, &candidate)) { return Err(denied()); }
            if Path::new(path).is_dir() {
                destructive::check(self, path)?;
            }
        }
        Ok(())
    }

    pub(super) fn write(&self, path: &str) -> io::Result<()> {
        self.read(path)?;
        if !self.access.allows_write() {
            return Err(io::Error::new(io::ErrorKind::ReadOnlyFilesystem, "Freigabe ist nur lesbar"));
        }
        if self.local && !self.allow_system_writes {
            if system_write(path) { return Err(denied()); }
            if self.local {
                if let Some(effective) = effective(path) {
                    if system_write(&effective.to_string_lossy()) { return Err(denied()); }
                }
            }
        }
        Ok(())
    }
}

fn private_roots() -> Vec<PathBuf> {
    let mut roots = vec![crate::support_dirs::app_data_dir()];
    if let Some(host) = crate::support_dirs::host() { roots.push(host.cache_dir.clone()); }
    roots
}
pub(super) fn ensure_handle_allowed(handle: &crate::local_access::DirectoryHandle) -> io::Result<()> {
    if handle.is_within_any(&private_roots())? { Err(denied()) } else { Ok(()) }
}

fn denied() -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, "Pfad ist nicht freigegeben") }

#[cfg(test)]
#[path = "fs_policy_task_tests.rs"]
mod task_tests;
