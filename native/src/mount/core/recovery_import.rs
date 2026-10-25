//! Pure validation of the versioned import contract.
use super::recovery_manifest::{validate_components, RecoveryManifest, RECOVERY_FORMAT_VERSION};
use std::{collections::HashSet, io};

pub(crate) fn validate_manifest(manifest: &RecoveryManifest) -> io::Result<()> {
    if manifest.version != RECOVERY_FORMAT_VERSION || manifest.import_version != RECOVERY_FORMAT_VERSION {
        return Err(io::Error::new(io::ErrorKind::Unsupported,
            "Recoveryformat oder Importversion wird nicht unterstützt"));
    }
    manifest.config.validate()?;
    if manifest.sealed != matches!(manifest.config.source, super::types::MountSource::Vault { .. }) {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "Recovery-Versiegelungsangabe widerspricht der Backendidentität"));
    }
    if matches!(manifest.config.source, super::types::MountSource::GoogleDrive { .. }) {
        super::recovery_identity::manifest_identity(&manifest.backend_identity, &manifest.config.source)?;
    } else if !manifest.sealed && manifest.backend_identity != serde_json::to_value(&manifest.config.source).map_err(io::Error::other)? {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "Recovery-Backend-/Rootidentität widerspricht der Mountkonfiguration"));
    }
    super::types::MountId::parse(&manifest.recovery_id)?;
    if !manifest.local_snapshot_complete {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "Recoverypaket besitzt keinen vollständigen Snapshot der verfügbaren lokalen Daten"));
    }
    let mut paths = HashSet::new();
    for file in &manifest.files {
        validate_components(&file.components)?;
        if !paths.insert(file.components.clone()) || file.sha256.len() != 64
            || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "Recoverymanifest besitzt doppelte Pfade oder ungültige SHA-256-Werte"));
        }
    }
    for directory in &manifest.directories {
        validate_components(directory)?;
        if !paths.insert(directory.clone()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "Recoverymanifest besitzt doppelte Verzeichnis-/Dateipfade"));
        }
    }
    let mount_id = manifest.config.id.as_str();
    if !manifest.directories.iter().any(|path| path.len() == 1 && path[0] == mount_id)
        && !manifest.gaps.iter().any(|gap| gap.kind == super::recovery_manifest::RecoveryGapKind::MissingCache
            && gap.subject == mount_id) {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "Recoverymanifest benennt weder den Mountcache noch dessen ausdrückliches Fehlen"));
    }
    Ok(())
}
