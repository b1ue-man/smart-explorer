//! Scope and live-session authority for persisted transfer names.
use std::{io, path::{Path, PathBuf}};
use crate::vault::{RootBinding, VaultLocator, VaultStoreLocator, vault_locked};
use super::{RegistryStore, hosts_register, register, mount_policy, vault_mount_of};

/// Never serialized and only redacted in Debug: bindings may contain names.
#[derive(Clone)]
pub(crate) struct TransferBlobContext {
    pub(super) bindings: Vec<RootBinding>,
    records: Vec<String>,
    epochs: Vec<u64>,
}
impl std::fmt::Debug for TransferBlobContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TransferBlobContext(..)")
    }
}

pub(crate) fn transfer_blob_context(locations: &[&str]) -> io::Result<Option<TransferBlobContext>> {
    let mut sites = Vec::new();
    for location in locations {
        let site = if let Ok(locator) = VaultLocator::parse(location) { Some(locator.site) }
        else if let Ok(locator) = VaultStoreLocator::parse(location) { Some(locator.site) }
        else if crate::vault::is_vault_locator(location) { return Err(invalid()); }
        else if !location.contains("://") { local_site(Path::new(location))? }
        else { None };
        if let Some(site) = site {
            let record = RegistryStore::open_default().resolve(&site).map_err(|_| invalid())?;
            sites.push(record.id);
        }
    }
    sites.sort(); sites.dedup();
    if sites.is_empty() { return Ok(None); }
    context_for_sites(&sites).map(Some)
}

pub(super) fn context_for_sites(sites: &[String]) -> io::Result<TransferBlobContext> {
    if sites.is_empty() || sites.windows(2).any(|pair| pair[0] >= pair[1]) { return Err(invalid()); }
    let mut context = TransferBlobContext { bindings: Vec::new(), records: Vec::new(), epochs: Vec::new() };
    for site in sites {
        if !crate::vault::is_site_id(site) { return Err(invalid()); }
        let epoch = session_epoch(site)?;
        let record = RegistryStore::open_default().resolve(site).map_err(|_| invalid())?;
        let binding = mount_policy(site).map_err(|_| invalid())?.binding;
        if binding.site != record.id || binding.vault != record.vault_id || binding.location != record.location { return Err(invalid()); }
        context.bindings.push(binding);
        context.records.push(record.serialize());
        context.epochs.push(epoch);
    }
    validate_records(&context)?;
    Ok(context)
}

/// Local-only cache guard: no provider resolution and no IPC. Outside the
/// register host there is no synchronous local proof, so hide cached names.
/// Consumers must clear plaintext on Err, and also on register lock events.
/// Full open validates current provider/account/root identity separately.
pub(crate) fn check_transfer_blob_context(context: &TransferBlobContext) -> io::Result<()> {
    validate_records(context)?;
    for (binding, expected) in context.bindings.iter().zip(&context.epochs) {
        if !hosts_register() { return Err(vault_locked(&binding.site)); }
        let (session, epoch) = register().session(&binding.site).ok_or_else(|| vault_locked(&binding.site))?;
        if epoch != *expected || session.config().jti != binding.vault { return Err(vault_locked(&binding.site)); }
    }
    Ok(())
}

/// Persisting an already captured job must remain possible during provider
/// outage. This checks only local records and session epochs; it never rebinds.
pub(crate) fn check_transfer_blob_context_live(context: &TransferBlobContext) -> io::Result<()> {
    validate_records(context)?;
    for (binding, expected) in context.bindings.iter().zip(&context.epochs) {
        if hosts_register() {
            let (session, _) = register().session(&binding.site).ok_or_else(|| vault_locked(&binding.site))?;
            if session.config().jti != binding.vault { return Err(vault_locked(&binding.site)); }
        }
        if session_epoch(&binding.site)? != *expected { return Err(vault_locked(&binding.site)); }
    }
    Ok(())
}
fn validate_records(context: &TransferBlobContext) -> io::Result<()> {
    for (binding, expected) in context.bindings.iter().zip(&context.records) {
        let record = RegistryStore::open_default().resolve(&binding.site).map_err(|_| invalid())?;
        if record.serialize() != *expected { return Err(invalid()); }
    }
    Ok(())
}
fn session_epoch(site: &str) -> io::Result<u64> {
    if hosts_register() {
        return register().session(site).map(|(_, epoch)| epoch).ok_or_else(|| vault_locked(site));
    }
    let status = crate::daemon::vault_status().map_err(|_| vault_locked(site))?.ok_or_else(|| vault_locked(site))?;
    status.sites.iter().find(|entry| entry.site == site && entry.state == "unlocked")
        .and_then(|entry| entry.epoch).ok_or_else(|| vault_locked(site))
}
fn local_site(path: &Path) -> io::Result<Option<String>> {
    let known = vault_mount_of(path);
    if let Some(Some(site)) = &known { return Ok(Some(site.clone())); }
    // Desktop clients need the worker's active mappings too. Do not start a
    // worker merely to classify an ordinary local transfer.
    if crate::daemon::vault_status().map_err(|_| invalid())?.is_some() {
        let mounts = crate::daemon::list_mounts().map_err(|_| invalid())?;
        let mut candidates = Vec::new();
        for mount in mounts {
            let crate::mount::MountSource::Vault { site_id } = mount.config.source else { continue; };
            let root = match mount.status {
                crate::mount::MountStatus::MountedAt { path } => path,
                crate::mount::MountStatus::Mounted { drive } | crate::mount::MountStatus::Conflict { drive, .. } => PathBuf::from(format!("{}:\\", drive.get())),
                _ => continue,
            };
            if crate::vault::core::klartext_policy::within(path, &root) { candidates.push(site_id); }
        }
        candidates.sort(); candidates.dedup();
        if candidates.len() > 1 { return Err(invalid()); }
        if let Some(site) = candidates.pop() { return Ok(Some(site)); }
    }
    if known.is_some() { Err(invalid()) } else { Ok(None) }
}
pub(super) fn invalid() -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, "Tresorbindung der Transfermetadaten ist nicht bestätigt") }

pub(super) use check_transfer_blob_context_live as validate_context;
