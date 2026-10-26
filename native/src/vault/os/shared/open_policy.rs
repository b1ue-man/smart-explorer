//! Persisted consent bound to the actual resolved storage, never to a scheme.
use std::io;
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::vault::core::open_policy::{stored_policy, store_policy, MountPolicy, RootBinding, ROOT_TRUST_REQUIRED};
use crate::vfs::{Backend, BackendHandle, DeleteDisposition};
use super::registry_store::RegistryStore;

fn binding(record: &crate::vault::SiteRecord, inner: &dyn Backend, root: &str) -> Result<RootBinding, String> {
    if record.vault_id.is_empty() {
        return Err("Der Tresor besitzt keine bestätigte Vaultidentität; bitte erneut eintragen".into());
    }
    let detection = super::manage::detect_storage(inner, root).map_err(|error| error.to_string())?;
    if detection.vault_id.as_deref() != Some(record.vault_id.as_str()) || detection.problem.is_some() {
        return Err("Am Speicherort liegt nicht der eingetragene Tresor".into());
    }
    let object = if inner.is_local() {
        Some(match crate::vfs::directory_identity(inner, root) {
            Ok(Some(identity)) => identity,
            _ => super::root_identity::identity(root).map_err(|error| error.to_string())?,
        })
    } else {
        inner.item_id(root).map_err(|error| error.to_string())?
    };
    Ok(RootBinding {
        site: record.id.clone(), vault: record.vault_id.clone(), location: record.location.clone(),
        backend: format!("{:?}", inner.scheme()), namespace: inner.namespace_identity(),
        account: inner.state_identity(), root: root.to_string(), object,
    })
}

pub(crate) fn mount_policy(site: &str) -> Result<MountPolicy, String> {
    let store = RegistryStore::open_default();
    let record = store.resolve(site)?;
    let (inner, root) = crate::connect::resolve_endpoint(&record.location)?;
    let inner = super::backend::confine_storage(inner, &root);
    policy_for(&record, inner.as_ref(), &root)
}

pub(super) fn policy_for(record: &crate::vault::SiteRecord, inner: &dyn Backend, root: &str) -> Result<MountPolicy, String> {
    let binding = binding(record, inner, root)?;
    let policy = stored_policy(record)?;
    let capabilities = inner.mount_path_capabilities(root).map_err(|error| error.to_string())?;
    Ok(MountPolicy {
        enforced: capabilities.root_confinement.is_enforced(),
        trusted: policy.trusted_root.as_ref() == Some(&binding),
        cleartext_copy: policy.cleartext_copy.as_ref() == Some(&binding),
        read_only: inner.delete_disposition() == DeleteDisposition::Unsupported
            || !capabilities.staged_write.supports_mounted_writes()
            || (inner.is_local() && crate::vfs::capacity(inner, root).is_ok_and(|capacity| capacity.read_only)),
        binding,
    })
}

/// No resolver is needed to revoke consent, including while the remote is down.
pub(crate) fn set_root_trust(site: &str, enabled: bool) -> Result<(), String> {
    set_policy(site, enabled, false)?;
    if !enabled {
        // Reuse the existing lock participants: flush or preserve sealed changes,
        // revoke Android descriptors, end drive consumers, then wipe session keys.
        let record = RegistryStore::open_default().resolve(site)?;
        if super::session::hosts_register() {
            super::session::register().lock(&record.id, super::session::LockRequest::new(
                crate::vault::LockReason::Explicit, true,
            )).map_err(|error| error.to_string())?;
        } else {
            crate::daemon::lock_vaults(Some(&record.id), true).map_err(|error| error.message)?;
        }
    }
    Ok(())
}

/// The opening dialog confirms this exact observation. A changed account,
/// object or locator must be shown and confirmed again rather than rebinding.
pub(crate) fn confirm_root_trust(expected: &RootBinding) -> Result<(), String> {
    let current = mount_policy(&expected.site)?;
    if &current.binding != expected {
        return Err("Die Speicheridentität wurde während der Bestätigung geändert; bitte erneut öffnen".into());
    }
    let store = RegistryStore::open_default();
    store.update(&expected.site, |record| {
        if record.location != expected.location || record.vault_id != expected.vault {
            return Err("Der Tresorort wurde während der Bestätigung geändert".into());
        }
        let mut policy = stored_policy(record)?;
        policy.trusted_root = Some(expected.clone());
        store_policy(record, &policy)
    })?;
    Ok(())
}

pub(crate) fn set_cleartext_copy_policy(site: &str, enabled: bool) -> Result<(), String> {
    set_policy(site, enabled, true)
}

fn set_policy(site: &str, enabled: bool, copy: bool) -> Result<(), String> {
    let store = RegistryStore::open_default();
    let record = store.resolve(site)?;
    let expected = if enabled { Some(mount_policy(&record.id)?.binding) } else { None };
    store.update(&record.id, |current| {
        if current.location != record.location || current.vault_id != record.vault_id {
            return Err("Der Tresorort wurde während der Bestätigung geändert; bitte erneut bestätigen".into());
        }
        let mut policy = stored_policy(current)?;
        if copy { policy.cleartext_copy = expected; } else { policy.trusted_root = expected; }
        store_policy(current, &policy)
    })?;
    Ok(())
}

/// Kept by the exact mount backend and its open streams. Checking the current
/// private record makes revocation effective without restarting the daemon.
#[derive(Clone)]
pub(crate) struct MountAccess {
    binding: RootBinding,
    dir: PathBuf,
    trusted: bool,
}

impl MountAccess {
    pub(crate) fn check(&self) -> io::Result<()> {
        let store = RegistryStore::at(self.dir.clone(), super::platform_lock_defaults());
        let record = store.get(&self.binding.site)?.ok_or_else(refused)?;
        if record.location != self.binding.location || record.vault_id != self.binding.vault {
            return Err(refused());
        }
        if self.trusted && stored_policy(&record).map_err(io::Error::other)?.trusted_root.as_ref() != Some(&self.binding) {
            return Err(refused());
        }
        if self.binding.backend == "Local" {
            let observed = if self.binding.object.as_deref().is_some_and(|identity| identity.starts_with("windows-full:") || identity.starts_with("unix-mounted:")) {
                crate::local_access::ConfinedDirectory::open_root(std::path::Path::new(&self.binding.root))?.identity()?
            } else { super::root_identity::identity(&self.binding.root)? };
            if self.binding.object.as_deref() != Some(observed.as_str()) { return Err(refused()); }
        }
        Ok(())
    }
}

fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("{ROOT_TRUST_REQUIRED}: Die Speicherfreigabe dieses Tresors fehlt oder wurde widerrufen"))
}

/// The backend must be the just-resolved cleartext view of this exact binding.
/// Config::Trusted alone is deliberately insufficient for vaults: old records
/// carry no historical account/root evidence and cannot safely migrate silently.
pub(crate) fn mount_access(backend: &BackendHandle, enforced: bool) -> io::Result<Option<MountAccess>> {
    let site = match super::local_store::vault_site_of(backend.as_ref()) {
        None => return Ok(None),
        Some(Some(site)) => site,
        Some(None) => return Err(refused()),
    };
    let policy = mount_policy(&site).map_err(io::Error::other)?;
    let identity = format!("vault:{}#{}", policy.binding.account, policy.binding.root);
    let namespace = format!("vault:{}#{}", policy.binding.namespace, policy.binding.root);
    if backend.state_identity() != identity || backend.namespace_identity() != namespace || (!enforced && !policy.trusted) {
        return Err(refused());
    }
    if enforced && policy.binding.backend == "Local" {
        let expected = policy.binding.object.as_ref().map(|identity| format!("vault-root:{}:{identity}", policy.binding.vault));
        if crate::vfs::directory_identity(backend.as_ref(), "/")? != expected { return Err(refused()); }
    }
    Ok(Some(MountAccess {
        binding: policy.binding, dir: RegistryStore::open_default().dir().to_path_buf(), trusted: !enforced,
    }))
}

/// The provider keeps this exact consent for its engine and every proxy handle,
/// checking it before cached cleartext as well as backend operations.
pub(crate) fn authorize_provider(backend: BackendHandle) -> io::Result<(BackendHandle, MountAccess)> {
    let enforced = backend.mount_path_capabilities("/")?.root_confinement.is_enforced();
    let access = mount_access(&backend, enforced)?.ok_or_else(refused)?;
    access.check()?;
    Ok((backend, access))
}

fn mounted_bindings() -> &'static Mutex<HashMap<String, RootBinding>> {
    static BINDINGS: OnceLock<Mutex<HashMap<String, RootBinding>>> = OnceLock::new();
    BINDINGS.get_or_init(Default::default)
}

pub(crate) fn note_mount_binding(id: &str, binding: RootBinding) {
    mounted_bindings().lock().unwrap_or_else(PoisonError::into_inner).insert(id.into(), binding);
}

pub(crate) fn mount_binding_matches(id: &str, binding: &RootBinding) -> bool {
    mounted_bindings().lock().unwrap_or_else(PoisonError::into_inner).get(id) == Some(binding)
}

pub(crate) fn forget_mount_binding(id: &str) {
    mounted_bindings().lock().unwrap_or_else(PoisonError::into_inner).remove(id);
}

/// Recovery metadata is an offline snapshot, never a fresh authorization or
/// confinement claim. It contains no key material and performs no resolution.
pub(crate) fn recovery_identity(site: &str) -> Result<serde_json::Value, String> {
    let record = RegistryStore::open_default().resolve(site)?;
    let (binding, status) = match stored_policy(&record) {
        Ok(policy) => {
            let status = match policy.trusted_root.as_ref() {
                None => "missing",
                Some(binding) if binding.site == record.id && binding.vault == record.vault_id && binding.location == record.location => "stored_unverified",
                Some(_) => "record_mismatch",
            };
            (policy.trusted_root, status)
        }
        Err(_) => (None, "invalid"),
    };
    Ok(serde_json::json!({
        "site_id": record.id, "vault_id": record.vault_id, "location": record.location,
        "root_binding": binding, "binding_status": status,
    }))
}

#[cfg(test)]
#[path = "batches_resolved_policy_task_tests.rs"]
mod batches_resolved_policy_task_tests;
