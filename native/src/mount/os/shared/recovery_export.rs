//! Private, byte-for-byte snapshots. No remote I/O and no spool open/cleanup.
use crate::local_access::{DirectoryHandle, EntryKind};
use crate::mount::recovery_export::{gap, inspect_journal, inspect_stages};
use crate::mount::recovery_manifest::*;
use crate::mount::{MountConfig, MountId};
use sha2::{Digest, Sha256};
use std::{ffi::OsStr, io::{self, Read, Write}, path::{Path, PathBuf}};

const MAX_STATE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn recovery_cache_ids(base: &Path, config: &MountConfig) -> io::Result<Vec<MountId>> {
    let mut ids = vec![config.id.clone()];
    if config.source.is_device_tree() {
        let root = match DirectoryHandle::open_root(base) {
            Ok(root) => root,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ids),
            Err(error) => return Err(error),
        };
        let tree = match root.open_child(OsStr::new(config.id.as_str())) {
            Ok(tree) => tree,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ids),
            Err(error) => return Err(error),
        };
        match tree.open_regular_child(OsStr::new("children.json")) {
            Ok(file) => {
                let value: serde_json::Value = serde_json::from_slice(&read_bounded(file)?)
                    .map_err(io::Error::other)?;
                if value["version"] != 1 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData,
                        "Gerätebaum-Recovery kann die Version von children.json nicht zuordnen"));
                }
                let children = value["children"].as_array().ok_or_else(||
                    io::Error::new(io::ErrorKind::InvalidData, "children.json enthält keine Kindzuordnung"))?;
                for child in children {
                    let id = child["id"].as_str().ok_or_else(|| io::Error::new(
                        io::ErrorKind::InvalidData, "Gerätebaum-Recovery enthält keine Kind-ID"))?;
                    ids.push(MountId::parse(id)?);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    ids.dedup();
    Ok(ids)
}

pub(crate) fn export_mount_recovery(
    base: &Path, config: &MountConfig, ids: &[MountId], destination: &Path,
    backend_identity: serde_json::Value,
) -> io::Result<(PathBuf, RecoveryManifest, String)> {
    if !destination.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "Recoveryziel muss ein absoluter vorhandener Ordner sein"));
    }
    let destination_root = DirectoryHandle::open_root(destination)?;
    // A destination inside the source could recursively copy itself.
    if destination_root.is_within_any(&[base.to_path_buf()])? {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "Recoveryziel darf nicht im Mountcache liegen"));
    }
    let id = MountId::new_random()?.as_str().to_string();
    let name = format!("mount-recovery-{id}");
    let pending_name = format!(".{name}.pending");
    let package = destination_root.create_private_child(OsStr::new(&pending_name))?;
    let pending_path = destination.join(&pending_name);
    let package_path = destination.join(name);
    let cache = package.create_private_child(OsStr::new("cache"))?;
    let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?.as_millis();
    let mut manifest = RecoveryManifest {
        version: RECOVERY_FORMAT_VERSION, import_version: RECOVERY_FORMAT_VERSION,
        recovery_id: id, created_unix_ms: u64::try_from(created).unwrap_or(u64::MAX),
        config: config.clone(), backend_identity,
        sealed: matches!(config.source, crate::mount::MountSource::Vault { .. }),
        files: Vec::new(), directories: Vec::new(), journals: Vec::new(), gaps: Vec::new(),
        local_snapshot_complete: false, retained_original_cache: base.to_string_lossy().into_owned(),
    };
    if manifest.sealed {
        match manifest.backend_identity["binding_status"].as_str() {
            Some("missing") | None => gap(&mut manifest, RecoveryGapKind::MissingBackendIdentity,
                config.id.as_str().into(), "Historische Vault-/Backend-/Rootbindung fehlt; versiegelte Daten bleiben gesichert, Reaktivierung verlangt eine belegte Identitätszuordnung"),
            Some("record_mismatch") => gap(&mut manifest, RecoveryGapKind::BackendIdentityConflict,
                config.id.as_str().into(), "Gespeicherte Rootbindung widerspricht dem Vaultrecord; keine automatische Neuzuordnung"),
            _ => {},
        }
    }
    let source = match DirectoryHandle::open_root(base) {
        Ok(source) => Some(source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    for id in ids {
        let input = source.as_ref().map_or_else(|| Err(io::Error::from(io::ErrorKind::NotFound)),
            |source| source.open_child(OsStr::new(id.as_str())));
        match input {
            Ok(directory) => {
                let target = cache.create_private_child(OsStr::new(id.as_str()))?;
                manifest.directories.push(vec![id.as_str().into()]);
                copy_tree(&directory, &target, vec![id.as_str().into()], &mut manifest)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => gap(&mut manifest,
                RecoveryGapKind::MissingCache, id.as_str().into(),
                "Mountcache fehlt; dieser Pfad enthält keine gesicherten lokalen Payloads"),
            Err(error) => return Err(error),
        }
    }
    if matches!(config.source, crate::mount::MountSource::GoogleDrive { .. }) {
        // Capture the immutable authority from the copied bytes, never from
        // the currently signed-in provider. Broken/old proof must not prevent
        // saving all raw bytes, but grants no automatic replay authority.
        manifest.backend_identity = serde_json::to_value(&config.source).map_err(io::Error::other)?;
        match super::recovery_identity_io::read_mount_recovery_identity(&pending_path.join("cache"), config) {
            Ok(Some(identity)) => manifest.backend_identity = serde_json::to_value(identity).map_err(io::Error::other)?,
            Ok(None) => gap(&mut manifest, RecoveryGapKind::MissingBackendIdentity,
                config.id.as_str().into(), &super::recovery_identity::missing().to_string()),
            Err(error) => gap(&mut manifest, RecoveryGapKind::BackendIdentityConflict,
                config.id.as_str().into(), &format!("Historische Kontobindung ist nicht auswertbar; Rohbytes bleiben gesichert, automatische Reaktivierung ist gesperrt: {error}")),
        }
    }
    inspect_saved_state(&cache, &mut manifest)?;
    manifest.files.sort_by(|left, right| left.components.cmp(&right.components));
    manifest.directories.sort();
    manifest.local_snapshot_complete = true;
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
    let mut file = package.create_file_new(OsStr::new("manifest.json"))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    crate::mount::recovery_checkpoint(crate::mount::RecoveryCommitPoint::ManifestSynced)?;
    drop(file);
    for directory in manifest.directories.iter().rev() {
        let mut path = pending_path.join("cache");
        for part in directory { path.push(part); }
        super::recovery_import_io::confirm_recovery_namespace(&path)?;
    }
    for path in [&pending_path.join("cache"), &pending_path, destination] {
        super::recovery_import_io::confirm_recovery_namespace(path)?;
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    drop(cache);
    drop(package);
    super::recovery_storage::publish(&pending_path, &package_path, false)?;
    super::recovery_import_io::verify_recovery_package(&package_path, Some(&digest))?;
    crate::mount::recovery_checkpoint(crate::mount::RecoveryCommitPoint::PackagePublished)?;
    Ok((package_path, manifest, digest))
}

fn copy_tree(source: &DirectoryHandle, destination: &DirectoryHandle, path: Vec<String>,
    manifest: &mut RecoveryManifest) -> io::Result<()> {
    if path.len() > 128 { return Err(io::Error::other("Recoverycache überschreitet 128 Verzeichnisebenen")); }
    for entry in source.read_directory()? {
        let entry = entry?;
        let name = entry.name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,
            "Mountcache enthält einen Dateinamen außerhalb des portablen Recoveryformats"))?;
        let mut child_path = path.clone();
        child_path.push(name.into());
        validate_components(&child_path)?;
        match entry.kind {
            EntryKind::Directory if !entry.is_link_like => {
                let input = source.open_child(&entry.name)?;
                let output = destination.create_private_child(&entry.name)?;
                manifest.directories.push(child_path.clone());
                copy_tree(&input, &output, child_path, manifest)?;
            }
            EntryKind::File if !entry.is_link_like => {
                let mut input = source.open_regular_child(&entry.name)?;
                let mut output = destination.create_file_new(&entry.name)?;
                let mut digest = Sha256::new();
                let mut length = 0u64;
                let mut buffer = vec![0u8; 128 * 1024];
                loop {
                    let read = input.read(&mut buffer)?;
                    if read == 0 { break; }
                    output.write_all(&buffer[..read])?;
                    digest.update(&buffer[..read]);
                    length = length.checked_add(read as u64).ok_or_else(||
                        io::Error::other("Recoverypayload ist zu groß"))?;
                }
                output.sync_all()?;
                crate::mount::recovery_checkpoint(crate::mount::RecoveryCommitPoint::PayloadSynced)?;
                manifest.files.push(RecoveryFile { components: child_path,
                    length, sha256: format!("{:x}", digest.finalize()) });
            }
            _ => return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                "Mountcache enthält einen Link, eine Junction oder eine Spezialdatei; Originalcache bleibt unverändert")),
        }
    }
    Ok(())
}

fn inspect_saved_state(cache: &DirectoryHandle, manifest: &mut RecoveryManifest) -> io::Result<()> {
    let files = manifest.files.clone();
    for file in &files {
        let name = file.components.last().map(String::as_str).unwrap_or_default();
        if !matches!(name, "journal.jsonl" | "journal.jsonl.compact-old" | "journal.jsonl.compact-new" | "stages.json") {
            continue;
        }
        let mut parent = cache.clone();
        for part in &file.components[..file.components.len() - 1] { parent = parent.open_child(OsStr::new(part))?; }
        // No semantic read is possible for sealed state. The already hashed
        // raw file may exceed the decoder limit and must still be savable.
        let bytes = if manifest.sealed { Vec::new() }
            else { read_bounded(parent.open_regular_child(OsStr::new(name))?)? };
        if name == "stages.json" { inspect_stages(manifest, &file.components.join("/"), &bytes); }
        else { inspect_journal(manifest, file.components.clone(), &bytes)?; }
    }
    let roots: Vec<_> = manifest.directories.iter().filter(|path| path.len() == 1).cloned().collect();
    for root in roots {
        if !files.iter().any(|file| file.components.starts_with(&root)
            && file.components.last().is_some_and(|name| name.starts_with("journal.jsonl"))) {
            gap(manifest, RecoveryGapKind::MissingJournal, root.join("/"),
                "Kein Originaljournal vorhanden; lokale Dateien sind gesichert, ihre Zuordnung bleibt ungeklärt");
        }
    }
    if manifest.sealed && !files.iter().any(|file| file.components.last().is_some_and(|name| name == "spool-key.wrapped")) {
        gap(manifest, RecoveryGapKind::MissingVaultKey, manifest.config.id.as_str().into(),
            "Gewickelter Spoolschlüssel fehlt; versiegelte Payloads werden unverändert erhalten");
    }
    Ok(())
}

pub(super) fn read_bounded(mut file: std::fs::File) -> io::Result<Vec<u8>> {
    if file.metadata()?.len() > MAX_STATE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Recoveryzustandsdatei überschreitet 64 MiB"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STATE_BYTES { return Err(io::Error::other("Recoveryzustandsdatei wuchs beim Lesen")); }
    Ok(bytes)
}
