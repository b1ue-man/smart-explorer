//! Active mounts and their inactive owner change in the SAME registry
//! document. Rollback readers ignore metadata but see the removed mount.
use crate::local_access::DirectoryHandle;
use crate::mount::{InactiveMountRecovery, MountConfig, MountId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    ffi::OsStr,
    io::{self, Write},
    path::Path,
};

const FIELD: &str = "inactive_recoveries";
const FILES: [&str; 2] = [super::REGISTRY_FILE, super::ext::SECOND_FILE];

#[derive(Serialize, Deserialize)]
struct RecoveryBook {
    version: u32,
    records: Vec<InactiveMountRecovery>,
}

pub(super) fn effective(
    directory: &Path,
    legacy: Vec<MountConfig>,
) -> io::Result<Vec<MountConfig>> {
    for record in list_in(directory)? {
        if record.owns_mount_cache && legacy.iter().any(|config| config.id == record.config.id) {
            return Err(invalid(
                "Mount-ID besitzt gleichzeitig aktive Registrierung und inaktiven Recoverybesitz",
            ));
        }
    }
    Ok(legacy)
}

pub(in crate::daemon) fn list() -> io::Result<Vec<InactiveMountRecovery>> {
    let mut records = list_in(&super::ensure_registry_directory()?)?;
    for record in &mut records {
        match crate::mount::read_recovery_manifest(&record.package_path) {
            Ok((manifest, digest)) if digest == record.manifest_sha256 => {
                record.gap_count = manifest.gaps.len() as u64;
                record.gaps = manifest.gaps;
            }
            Ok(_) => {
                record.package_error =
                    Some("Manifest stimmt nicht mit dem gespeicherten SHA-256 überein".into())
            }
            Err(error) => {
                record.package_error = Some(format!(
                    "Vollständige Lückenliste ist derzeit nicht lesbar: {error}"
                ))
            }
        }
    }
    Ok(records)
}

pub(in crate::daemon) fn is_inactive(id: &MountId) -> io::Result<bool> {
    is_inactive_in(&super::ensure_registry_directory()?, id)
}

pub(in crate::daemon) fn is_inactive_in(directory: &Path, id: &MountId) -> io::Result<bool> {
    Ok(list_in(directory)?
        .iter()
        .any(|record| record.owns_mount_cache && record.config.id == *id))
}

pub(in crate::daemon) fn detach(record: InactiveMountRecovery) -> io::Result<()> {
    let directory = super::ensure_registry_directory()?;
    let records = list_in(&directory)?;
    if records.iter().any(|known| {
        known.id == record.id || (known.owns_mount_cache && known.config.id == record.config.id)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Laufwerk besitzt bereits eine inaktive Recovery",
        ));
    }
    let file_name = file_holding(&directory, &record.config)?;
    let mut document = read_document(&directory, file_name)?;
    let mounts = document["mounts"]
        .as_array_mut()
        .ok_or_else(|| invalid("Registry enthält keine Mountliste"))?;
    if let Some(value) = mounts
        .iter()
        .find(|value| value["id"].as_str() == Some(record.config.id.as_str()))
    {
        let stored: MountConfig =
            serde_json::from_value(value.clone()).map_err(io::Error::other)?;
        if stored != record.config {
            return Err(invalid(
                "Aktive Mountkonfiguration wurde während der Recovery geändert",
            ));
        }
    }
    mounts.retain(|value| value["id"].as_str() != Some(record.config.id.as_str()));
    let mut book = book(&document)?;
    book.records.push(record);
    document[FIELD] = serde_json::to_value(book).map_err(io::Error::other)?;
    write_document(&directory, file_name, &document)?;
    reconcile(&directory);
    Ok(())
}

pub(in crate::daemon) fn import(record: InactiveMountRecovery) -> io::Result<()> {
    let directory = super::ensure_registry_directory()?;
    if list_in(&directory)?
        .iter()
        .any(|known| known.id == record.id)
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Recovery-ID ist bereits vorhanden",
        ));
    }
    let file_name = file_for(&record.config);
    let mut document = read_document(&directory, file_name)?;
    let mut book = book(&document)?;
    book.records.push(record);
    document[FIELD] = serde_json::to_value(book).map_err(io::Error::other)?;
    write_document(&directory, file_name, &document)
}

pub(in crate::daemon) fn reactivate(config: &MountConfig) -> io::Result<()> {
    let directory = super::ensure_registry_directory()?;
    let active = super::load_in(&directory)?;
    if active.len() >= super::MAX_MOUNTS || active.iter().any(|known| known.id == config.id) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Mount-ID ist aktiv oder die Registry ist voll",
        ));
    }
    let file_name = file_for_reactivation(&directory, config)?;
    let mut document = read_document(&directory, file_name)?;
    let mut book = book(&document)?;
    for record in &mut book.records {
        if record.config.id == config.id {
            record.owns_mount_cache = false;
            record.reactivated = true;
        }
    }
    document[FIELD] = serde_json::to_value(book).map_err(io::Error::other)?;
    document["mounts"]
        .as_array_mut()
        .ok_or_else(|| invalid("Registry enthält keine Mountliste"))?
        .push(serde_json::to_value(config).map_err(io::Error::other)?);
    write_document(&directory, file_name, &document)?;
    reconcile(&directory);
    Ok(())
}

/// Ordinary legacy-compatible writers preserve the ownership field.
pub(super) fn preserve_field(
    directory: &Path,
    file_name: &str,
    body: &[u8],
) -> io::Result<Vec<u8>> {
    if !FILES.contains(&file_name) {
        return Ok(body.to_vec());
    }
    let Some(previous) = super::read_bounded(&directory.join(file_name))? else {
        return Ok(body.to_vec());
    };
    let previous: Value = serde_json::from_slice(&previous).map_err(io::Error::other)?;
    let Some(field) = previous.get(FIELD) else {
        return Ok(body.to_vec());
    };
    book(&previous)?;
    let mut document: Value = serde_json::from_slice(body).map_err(io::Error::other)?;
    document[FIELD] = field.clone();
    serde_json::to_vec(&document).map_err(io::Error::other)
}

pub(super) fn list_in(directory: &Path) -> io::Result<Vec<InactiveMountRecovery>> {
    let mut records = Vec::new();
    let mut ids = HashSet::new();
    let mut owners = HashSet::new();
    for file_name in FILES {
        for record in book(&read_document(directory, file_name)?)?.records {
            MountId::parse(&record.id)?;
            record.config.validate()?;
            if !record.package_path.is_absolute()
                || !record.retained_cache.is_absolute()
                || !ids.insert(record.id.clone())
                || (record.owns_mount_cache && !owners.insert(record.config.id.clone()))
            {
                return Err(invalid(
                    "Recoverybestand enthält ungültige oder widersprüchliche Eigentümer",
                ));
            }
            records.push(record);
        }
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(records)
}

fn book(document: &Value) -> io::Result<RecoveryBook> {
    let Some(value) = document.get(FIELD) else {
        return Ok(RecoveryBook {
            version: 1,
            records: Vec::new(),
        });
    };
    let result: RecoveryBook = serde_json::from_value(value.clone()).map_err(io::Error::other)?;
    if result.version != 1 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Nicht unterstützte Recovery-Eigentümerversion",
        ));
    }
    Ok(result)
}

fn read_document(directory: &Path, file_name: &str) -> io::Result<Value> {
    let Some(bytes) = super::read_bounded(&directory.join(file_name))? else {
        return Ok(serde_json::json!({"version":1,"mounts":[]}));
    };
    let document: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if document["version"] != 1 || !document["mounts"].is_array() {
        return Err(invalid("Ungültiges Mountregistrydokument"));
    }
    Ok(document)
}

fn file_holding(directory: &Path, config: &MountConfig) -> io::Result<&'static str> {
    for name in FILES {
        if read_document(directory, name)?["mounts"]
            .as_array()
            .is_some_and(|mounts| {
                mounts
                    .iter()
                    .any(|value| value["id"].as_str() == Some(config.id.as_str()))
            })
        {
            return Ok(name);
        }
    }
    Ok(file_for(config))
}

fn file_for(config: &MountConfig) -> &'static str {
    if config.source.stored_in_first_registry() {
        FILES[0]
    } else {
        FILES[1]
    }
}

fn file_for_reactivation(directory: &Path, config: &MountConfig) -> io::Result<&'static str> {
    // A legacy hand-edited kind may still have lived in registry.json.
    // Its original owner must be cleared in that same document atomically.
    for name in FILES {
        for record in book(&read_document(directory, name)?)?.records {
            if record.owns_mount_cache && record.config.id == config.id {
                if record.config.source != config.source {
                    return Err(invalid(
                        "Recovery-Mount-ID ist an eine abweichende Backend-/Rootidentität gebunden",
                    ));
                }
                return Ok(name);
            }
        }
    }
    Ok(file_for(config))
}

fn reconcile(directory: &Path) {
    if let Ok(configs) = super::load_in(directory) {
        super::super::drive_autostart::reconcile(&configs);
    }
}

fn write_document(directory: &Path, file_name: &str, document: &Value) -> io::Result<()> {
    // Large delete/stage inventories live in the hash-bound manifest, not
    // duplicated inside the legacy 1-MiB registry. Listing hydrates them.
    let mut stored = document.clone();
    // Mutable indexing would insert a null records field on this lookup.
    // A legacy active document must keep its optional recovery field absent.
    if let Some(records) = stored
        .get_mut(FIELD)
        .and_then(|book| book.get_mut("records"))
        .and_then(Value::as_array_mut)
    {
        for record in records {
            record["gaps"] = serde_json::json!([]);
            if let Some(object) = record.as_object_mut() {
                object.remove("package_error");
            }
        }
    }
    let bytes = serde_json::to_vec(&stored).map_err(io::Error::other)?;
    if bytes.len() as u64 > super::MAX_REGISTRY_BYTES {
        return Err(invalid("Recoverybestand überschreitet das Registrylimit"));
    }
    let previous = super::read_bounded(&directory.join(file_name))?;
    match replace_private(directory, file_name, &bytes) {
        Ok(()) => Ok(()),
        Err(error) => {
            // A failed parent flush can follow a completed rename. Restore
            // the complete previous document before reporting failure.
            if super::read_bounded(&directory.join(file_name))?.as_deref() == Some(bytes.as_slice())
            {
                let restored = match previous {
                    Some(previous) => replace_private(directory, file_name, &previous),
                    None => {
                        DirectoryHandle::open_root(directory)?
                            .remove_child(OsStr::new(file_name))?;
                        crate::mount::confirm_recovery_namespace(directory)
                    }
                };
                if let Err(rollback) = restored {
                    return Err(io::Error::other(format!(
                    "Recovery-Eigentumscommit ist unbestätigt: {error}; ursprüngliches Registrydokument wiederherstellen: {rollback}. Paket und Originalcache bleiben erhalten")));
                }
            }
            Err(error)
        }
    }
}

fn replace_private(directory: &Path, file_name: &str, bytes: &[u8]) -> io::Result<()> {
    let parent = DirectoryHandle::open_root(directory)?;
    parent.secure_private()?;
    let name = format!(".recovery-owner-{}.tmp", MountId::new_random()?);
    let mut file = parent.create_file_new(OsStr::new(&name))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    crate::mount::recovery_checkpoint(crate::mount::RecoveryCommitPoint::OwnershipSynced)?;
    drop(file);
    crate::mount::publish_recovery_object(&directory.join(name), &directory.join(file_name), true)?;
    crate::mount::recovery_checkpoint(crate::mount::RecoveryCommitPoint::OwnershipPublished)
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}

#[cfg(test)]
#[path = "batches_recovery_owner_task_tests.rs"]
mod batches_recovery_owner_task_tests;
