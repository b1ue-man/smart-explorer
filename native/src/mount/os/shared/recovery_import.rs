//! Verifies every saved byte before import/reactivation. Original caches
//! are never overwritten or removed, including when a package has gaps.
use crate::local_access::{DirectoryHandle, EntryKind};
use crate::mount::recovery_import::validate_manifest;
use crate::mount::{InactiveMountRecovery, RecoveryManifest};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub(crate) fn confirm_recovery_namespace(path: &Path) -> io::Result<()> {
    super::recovery_storage::confirm_namespace(path)
}

pub(crate) fn read_recovery_manifest(path: &Path) -> io::Result<(RecoveryManifest, String)> {
    let root = DirectoryHandle::open_root(path)?;
    let bytes = super::recovery_export_io::read_bounded(
        root.open_regular_child(OsStr::new("manifest.json"))?,
    )?;
    let mut manifest =
        serde_json::from_slice::<RecoveryManifest>(&bytes).map_err(io::Error::other)?;
    validate_manifest(&manifest)?;
    crate::mount::recovery_export::sealed_inventory_view(&mut manifest);
    Ok((manifest, format!("{:x}", Sha256::digest(&bytes))))
}

pub(crate) fn verify_recovery_package(
    path: &Path,
    expected_digest: Option<&str>,
) -> io::Result<RecoveryManifest> {
    let (manifest, digest) = read_recovery_manifest(path)?;
    if expected_digest.is_some_and(|expected| expected != digest) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Recoverymanifest wurde verändert",
        ));
    }
    let root = DirectoryHandle::open_root(path)?.open_child(OsStr::new("cache"))?;
    verify_tree(&root, &manifest)?;
    if matches!(
        manifest.config.source,
        crate::mount::MountSource::GoogleDrive { .. }
    ) {
        if let Some(expected) = crate::mount::recovery_manifest_identity(
            &manifest.backend_identity,
            &manifest.config.source,
        )? {
            let actual =
                crate::mount::read_mount_recovery_identity(&path.join("cache"), &manifest.config)?;
            if actual.as_ref() != Some(&expected) {
                return Err(crate::mount::recovery_identity_conflict());
            }
        }
    }
    let ids = super::recovery_export_io::recovery_cache_ids(&path.join("cache"), &manifest.config)?;
    if manifest
        .directories
        .iter()
        .filter(|components| components.len() == 1)
        .any(|components| !ids.iter().any(|id| id.as_str() == components[0]))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Recoverypaket enthält einen Cache außerhalb seiner Mount-/Gerätebaumidentitäten",
        ));
    }
    Ok(manifest)
}

pub(crate) fn copy_recovery_package(
    source: &Path,
    destination: &Path,
) -> io::Result<(PathBuf, RecoveryManifest, String)> {
    let manifest = verify_recovery_package(source, None)?;
    let (_, digest) = read_recovery_manifest(source)?;
    let parent = DirectoryHandle::open_root(destination)?;
    let name = format!("mount-recovery-{}", manifest.recovery_id);
    let pending_name = format!(".{name}-{}.pending", crate::mount::MountId::new_random()?);
    let output = parent.create_private_child(OsStr::new(&pending_name))?;
    let input = DirectoryHandle::open_root(source)?;
    let source_cache = input.open_child(OsStr::new("cache"))?;
    let target_cache = output.create_private_child(OsStr::new("cache"))?;
    copy_verified(&source_cache, &target_cache)?;
    let mut reader = input.open_regular_child(OsStr::new("manifest.json"))?;
    let mut writer = output.create_file_new(OsStr::new("manifest.json"))?;
    io::copy(&mut reader, &mut writer)?;
    writer.sync_all()?;
    drop(writer);
    let temporary = destination.join(&pending_name);
    sync_tree_namespaces(&temporary.join("cache"), &manifest)?;
    confirm_recovery_namespace(&temporary)?;
    drop(target_cache);
    drop(output);
    let final_path = destination.join(name);
    super::recovery_storage::publish(&temporary, &final_path, false)?;
    verify_recovery_package(&final_path, Some(&digest))?;
    Ok((final_path, manifest, digest))
}

fn verify_tree(root: &DirectoryHandle, manifest: &RecoveryManifest) -> io::Result<()> {
    let mut observed = BTreeMap::new();
    inventory(root, Vec::new(), &mut observed)?;
    let expected: BTreeMap<_, _> = manifest
        .files
        .iter()
        .map(|file| {
            (
                file.components.clone(),
                Some((file.length, file.sha256.clone())),
            )
        })
        .chain(
            manifest
                .directories
                .iter()
                .cloned()
                .map(|path| (path, None)),
        )
        .collect();
    if observed != expected {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "Recovery-Dateien, Verzeichnisse, Längen oder SHA-256-Werte stimmen nicht mit dem Manifest überein"));
    }
    Ok(())
}

fn inventory(
    root: &DirectoryHandle,
    path: Vec<String>,
    result: &mut BTreeMap<Vec<String>, Option<(u64, String)>>,
) -> io::Result<()> {
    if path.len() > 128 {
        return Err(io::Error::other(
            "Recoveryinventar überschreitet 128 Verzeichnisebenen",
        ));
    }
    for entry in root.read_directory()? {
        let entry = entry?;
        let mut child_path = path.clone();
        child_path.push(
            entry
                .name
                .to_str()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Recovery enthält einen nicht darstellbaren Dateinamen",
                    )
                })?
                .into(),
        );
        crate::mount::recovery_manifest::validate_components(&child_path)?;
        match entry.kind {
            EntryKind::Directory if !entry.is_link_like => {
                result.insert(child_path.clone(), None);
                inventory(&root.open_child(&entry.name)?, child_path, result)?;
            }
            EntryKind::File if !entry.is_link_like => {
                let mut file = root.open_regular_child(&entry.name)?;
                let mut hash = Sha256::new();
                let mut length = 0u64;
                let mut buffer = vec![0u8; 128 * 1024];
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    hash.update(&buffer[..count]);
                    length = length
                        .checked_add(count as u64)
                        .ok_or_else(|| io::Error::other("Recoverylänge läuft über"))?;
                }
                result.insert(child_path, Some((length, format!("{:x}", hash.finalize()))));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Recovery enthält einen Link, eine Junction oder eine Spezialdatei",
                ))
            }
        }
    }
    Ok(())
}

/// An existing retained cache must match the snapshot exactly. Imported
/// caches are installed only into absent IDs, never over a user's files.
pub(crate) fn restore_recovery_cache(
    record: &InactiveMountRecovery,
    base: &Path,
) -> io::Result<RecoveryManifest> {
    let manifest = verify_recovery_package(&record.package_path, Some(&record.manifest_sha256))?;
    let root = match DirectoryHandle::open_root(base) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent_path = base
                .parent()
                .ok_or_else(|| io::Error::other("Cachebasis hat keinen Parent"))?;
            let parent = DirectoryHandle::open_root(parent_path)?;
            let pending = format!(
                ".mount-cache-{}.pending",
                crate::mount::MountId::new_random()?
            );
            drop(parent.create_private_child(OsStr::new(&pending))?);
            super::recovery_storage::publish(&parent_path.join(pending), base, false)?;
            DirectoryHandle::open_root(base)?
        }
        Err(error) => return Err(error),
    };
    let source =
        DirectoryHandle::open_root(&record.package_path)?.open_child(OsStr::new("cache"))?;
    let roots: Vec<_> = manifest
        .directories
        .iter()
        .filter(|path| path.len() == 1)
        .collect();
    // Check every occupied target before creating even one missing target.
    for path in &roots {
        match root.open_child(OsStr::new(&path[0])) {
            Ok(target) => verify_subtree(&target, path, &manifest)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for path in roots {
        match root.open_child(OsStr::new(&path[0])) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let input = source.open_child(OsStr::new(&path[0]))?;
        let pending = format!(
            ".restore-{}-{}.pending",
            path[0],
            crate::mount::MountId::new_random()?
        );
        let target = root.create_private_child(OsStr::new(&pending))?;
        copy_verified(&input, &target)?;
        verify_subtree(&target, path, &manifest)?;
        let stage = base.join(&pending);
        for directory in manifest
            .directories
            .iter()
            .filter(|directory| directory.starts_with(path))
        {
            let mut target = stage.clone();
            for part in &directory[1..] {
                target.push(part);
            }
            confirm_recovery_namespace(&target)?;
        }
        drop(target);
        super::recovery_storage::publish(&stage, &base.join(&path[0]), false)?;
    }
    confirm_recovery_namespace(base)?;
    Ok(manifest)
}

fn sync_tree_namespaces(base: &Path, manifest: &RecoveryManifest) -> io::Result<()> {
    for directory in manifest.directories.iter().rev() {
        let mut target = base.to_path_buf();
        for part in directory {
            target.push(part);
        }
        confirm_recovery_namespace(&target)?;
    }
    confirm_recovery_namespace(base)
}

fn verify_subtree(
    root: &DirectoryHandle,
    prefix: &[String],
    manifest: &RecoveryManifest,
) -> io::Result<()> {
    let mut observed = BTreeMap::new();
    observed.insert(prefix.to_vec(), None);
    inventory(root, prefix.to_vec(), &mut observed)?;
    let expected: BTreeMap<_, _> = manifest
        .files
        .iter()
        .filter(|file| file.components.starts_with(prefix))
        .map(|file| {
            (
                file.components.clone(),
                Some((file.length, file.sha256.clone())),
            )
        })
        .chain(
            manifest
                .directories
                .iter()
                .filter(|path| path.starts_with(prefix))
                .cloned()
                .map(|path| (path, None)),
        )
        .collect();
    if observed != expected {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Vorhandener Cache weicht von der Recovery ab; beide Bestände bleiben unverändert",
        ));
    }
    Ok(())
}

fn copy_verified(input: &DirectoryHandle, output: &DirectoryHandle) -> io::Result<()> {
    for entry in input.read_directory()? {
        let entry = entry?;
        match entry.kind {
            EntryKind::Directory if !entry.is_link_like => copy_verified(
                &input.open_child(&entry.name)?,
                &output.create_private_child(&entry.name)?,
            )?,
            EntryKind::File if !entry.is_link_like => {
                let mut reader = input.open_regular_child(&entry.name)?;
                let mut writer = output.create_file_new(&entry.name)?;
                io::copy(&mut reader, &mut writer)?;
                writer.flush()?;
                writer.sync_all()?;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Recoveryobjekt ist kein regulärer Eintrag",
                ))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "batches_recovery_task_tests.rs"]
mod batches_recovery_task_tests;
