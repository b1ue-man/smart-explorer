//! Offline save/detach is separate from Stop: a running host is refused,
//! never killed or asked to publish before the local snapshot is durable.
use super::{registered_entry, snapshot, MountManager, RESTART_DETAIL};
use crate::mount::{InactiveMountRecovery, MountId, MountRecovery, MountSnapshot};
use std::{
    any::Any,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

impl MountManager {
    pub(in crate::daemon) fn save_remove(
        &self,
        id: &MountId,
        destination: &Path,
    ) -> Result<InactiveMountRecovery, String> {
        let _lifecycle = self.lifecycle_guard()?;
        self.ensure_registry_ready()?;
        ensure_desktop_recovery()?;
        let config = {
            let state = self.state_guard()?;
            let entry = state
                .get(id.as_str())
                .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
            if entry.child.is_some() || entry.backend_stream_active {
                return Err("Sichern und entfernen verweigert: Der Laufwerkhost oder ein Remote-Aufruf läuft noch. Offene Dateien schließen und das Laufwerk zuerst auswerfen; Änderungen bleiben dabei im Cache".into());
            }
            entry.config.clone()
        };
        let base = crate::support_dirs::app_data_dir().join("mount-cache");
        let ids = crate::mount::recovery_cache_ids(&base, &config).map_err(detail)?;
        let _leases = lease_existing(&base, &ids).map_err(detail)?;
        let identity = match &config.source {
            crate::mount::MountSource::Vault { site_id } => crate::vault::recovery_identity(
                site_id,
            )
            .unwrap_or_else(|_| serde_json::json!({"site_id":site_id,"binding_status":"missing"})),
            source => {
                serde_json::to_value(source).map_err(|error| detail(io::Error::other(error)))?
            }
        };
        let (package_path, manifest, manifest_sha256) =
            crate::mount::export_mount_recovery(&base, &config, &ids, destination, identity)
                .map_err(detail)?;
        let recovery = InactiveMountRecovery {
            id: manifest.recovery_id.clone(),
            config,
            package_path,
            manifest_sha256,
            retained_cache: base,
            cache_ids: ids,
            reactivated: false,
            owns_mount_cache: true,
            sealed: manifest.sealed,
            local_snapshot_complete: manifest.local_snapshot_complete,
            gap_count: manifest.gaps.len() as u64,
            gaps: manifest.gaps,
            package_error: None,
        };
        // Hold state across the only ownership commit. No subsequent fallible
        // state lock can leave a successfully detached entry active in memory.
        let mut state = self.state_guard()?;
        super::super::mount_registry::inactive::detach(recovery.clone()).map_err(detail)?;
        state.remove(id.as_str());
        crate::vault::forget_mount_binding(id.as_str());
        Ok(recovery)
    }

    pub(in crate::daemon) fn recovery_list(&self) -> Result<Vec<InactiveMountRecovery>, String> {
        let _lifecycle = self.lifecycle_guard()?;
        ensure_desktop_recovery()?;
        super::super::mount_registry::inactive::list().map_err(detail)
    }

    pub(in crate::daemon) fn recovery_import(
        &self,
        package: &Path,
    ) -> Result<InactiveMountRecovery, String> {
        let _lifecycle = self.lifecycle_guard()?;
        ensure_desktop_recovery()?;
        self.ensure_registry_ready()?;
        // Copy into application-owned private storage: unplugging the chosen
        // import source must not orphan an apparently imported recovery.
        let parent = ensure_recovery_directory().map_err(detail)?;
        let (package_path, imported, manifest_sha256) =
            crate::mount::copy_recovery_package(package, &parent).map_err(detail)?;
        let record = InactiveMountRecovery {
            retained_cache: package_path.join("cache"),
            cache_ids: imported
                .directories
                .iter()
                .filter(|path| path.len() == 1)
                .map(|path| MountId::parse(&path[0]))
                .collect::<io::Result<_>>()
                .map_err(detail)?,
            id: imported.recovery_id,
            config: imported.config,
            package_path,
            reactivated: false,
            owns_mount_cache: false,
            manifest_sha256,
            sealed: imported.sealed,
            local_snapshot_complete: imported.local_snapshot_complete,
            gap_count: imported.gaps.len() as u64,
            gaps: imported.gaps,
            package_error: None,
        };
        super::super::mount_registry::inactive::import(record.clone()).map_err(detail)?;
        Ok(record)
    }

    pub(in crate::daemon) fn recovery_reactivate(
        &self,
        recovery_id: &str,
    ) -> Result<MountSnapshot, String> {
        let _lifecycle = self.lifecycle_guard()?;
        ensure_desktop_recovery()?;
        self.ensure_registry_ready()?;
        let record = super::super::mount_registry::inactive::list()
            .map_err(detail)?
            .into_iter()
            .find(|record| record.id == recovery_id)
            .ok_or_else(|| "Recovery-ID ist nicht vorhanden".to_string())?;
        let mut state = self.state_guard()?;
        if state.contains_key(record.config.id.as_str()) {
            return Err(
                "Die Mount-ID ist bereits aktiv; vorhandenes Laufwerk bleibt unverändert".into(),
            );
        }
        let base = crate::support_dirs::app_data_dir().join("mount-cache");
        let (identity_manifest, digest) =
            crate::mount::read_recovery_manifest(&record.package_path).map_err(detail)?;
        if digest != record.manifest_sha256 {
            return Err(
                "Recoverymanifest wurde verändert; beide Bestände bleiben unverändert".into(),
            );
        }
        if matches!(
            identity_manifest.config.source,
            crate::mount::MountSource::GoogleDrive { .. }
        ) && crate::mount::recovery_manifest_identity(
            &identity_manifest.backend_identity,
            &identity_manifest.config.source,
        )
        .map_err(detail)?
        .is_none()
        {
            return Err(crate::mount::recovery_identity_missing().to_string());
        }
        validate_vault_identity(&identity_manifest)?;
        let manifest = crate::mount::restore_recovery_cache(&record, &base).map_err(detail)?;
        let mut config = manifest.config;
        config.auto_remount = false;
        if matches!(config.source, crate::mount::MountSource::Vault { .. })
            && manifest.backend_identity["root_binding"].is_null()
        {
            // An absent historical trust binding grants no Trusted flag.
            // Retry resolves/validates this exact vault through P21 again.
            config.root_security = crate::mount::MountRootSecurity::Enforced;
        }
        super::super::mount_registry::inactive::reactivate(&config).map_err(detail)?;
        let entry = registered_entry(config, MountRecovery::Unknown, RESTART_DETAIL.into());
        let result = snapshot(&entry);
        state.insert(entry.config.id.as_str().into(), entry);
        Ok(result)
    }
}

fn detail(error: io::Error) -> String {
    format!("Laufwerk-Recovery: {error}")
}

fn ensure_desktop_recovery() -> Result<(), String> {
    if crate::mount::drive_mount_supported() {
        Ok(())
    } else {
        Err("Diese Plattform besitzt keine Desktop-Mountregistry; Laufwerk-Recovery wird nur unter Windows und Linux angeboten".into())
    }
}

fn validate_vault_identity(manifest: &crate::mount::RecoveryManifest) -> Result<(), String> {
    let crate::mount::MountSource::Vault { site_id } = &manifest.config.source else {
        return Ok(());
    };
    let expected = &manifest.backend_identity;
    if expected["binding_status"] == "record_mismatch"
        || expected["vault_id"].as_str().is_none_or(str::is_empty)
        || expected["location"].as_str().is_none_or(str::is_empty)
    {
        return Err("Vault-Recovery besitzt keine eindeutige historische Vault-/Ortidentität. Paket und Originalcache bleiben erhalten; Vaultidentität ausdrücklich zuordnen, bevor Änderungen veröffentlicht werden".into());
    }
    let current = crate::vault::recovery_identity(site_id)?;
    if ["site_id", "vault_id", "location"]
        .iter()
        .any(|field| expected[*field] != current[*field])
        || (!expected["root_binding"].is_null()
            && expected["root_binding"] != current["root_binding"])
    {
        return Err("Die lokale Vault-/Backend-/Rootidentität widerspricht dem Recoverypaket; beide Bestände bleiben unverändert".into());
    }
    Ok(())
}

fn ensure_recovery_directory() -> io::Result<PathBuf> {
    let appdata = crate::support_dirs::app_data_dir();
    let parent = crate::local_access::DirectoryHandle::open_root(&appdata)?;
    let name = OsStr::new("mount-recovery");
    match parent.open_child(name) {
        Ok(directory) => directory.secure_private()?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let pending = format!(".mount-recovery-{}.pending", MountId::new_random()?);
            drop(parent.create_private_child(OsStr::new(&pending))?);
            crate::mount::publish_recovery_object(
                &appdata.join(pending),
                &appdata.join(name),
                false,
            )?;
        }
        Err(error) => return Err(error),
    }
    crate::mount::confirm_recovery_namespace(&appdata)?;
    Ok(appdata.join(name))
}

fn lease_existing(base: &Path, ids: &[MountId]) -> io::Result<Vec<Box<dyn Any + Send>>> {
    let root = match crate::local_access::DirectoryHandle::open_root(base) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut leases = Vec::new();
    for id in ids {
        let directory = match root.open_child(OsStr::new(id.as_str())) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        match directory.open_child(OsStr::new("files")) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
        #[cfg(windows)]
        let lease = crate::mount::os::windows::CacheLease::acquire(base, id)?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let lease = crate::mount::os::linux_os::CacheLease::acquire(base, id)?;
        #[cfg(any(windows, target_os = "linux", target_os = "android"))]
        leases.push(Box::new(lease) as Box<dyn Any + Send>);
    }
    Ok(leases)
}
