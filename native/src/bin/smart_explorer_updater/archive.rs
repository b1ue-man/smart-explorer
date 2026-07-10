use std::io::Write;
use std::path::{Path, PathBuf};

use super::hash::verify_sha256;
use super::replace::ReplaceTargetError;

pub(crate) fn archive_current_app(
    target: &Path,
    expected_sha256: &str,
    archive: &Path,
) -> Result<(), ReplaceTargetError> {
    verify_sha256(target, expected_sha256).map_err(ReplaceTargetError::integrity)?;
    let sidecar = sidecar_path(archive);
    if archive.exists()
        && sidecar.exists()
        && verify_sha256(archive, expected_sha256).is_ok()
        && sidecar_matches(&sidecar, expected_sha256)
    {
        return Ok(());
    }

    let parent = archive.parent().ok_or_else(|| {
        ReplaceTargetError::integrity("Archivpfad hat keinen Elternordner".to_string())
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        ReplaceTargetError::io(format!("Archivordner {} anlegen", parent.display()), error)
    })?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| ReplaceTargetError::io("Archiv temporaer anlegen", error))?;
    let mut source = std::fs::File::open(target).map_err(|error| {
        ReplaceTargetError::io(format!("Programmdatei {} lesen", target.display()), error)
    })?;
    std::io::copy(&mut source, temp.as_file_mut())
        .map_err(|error| ReplaceTargetError::io("Programmdatei archivieren", error))?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|error| ReplaceTargetError::io("Archiv synchronisieren", error))?;
    verify_sha256(temp.path(), expected_sha256).map_err(ReplaceTargetError::integrity)?;
    temp.persist(archive).map_err(|error| {
        ReplaceTargetError::io(
            format!("Archiv {} atomar einsetzen", archive.display()),
            error.error,
        )
    })?;
    write_atomic(&sidecar, format!("{}\n", expected_sha256).as_bytes())?;
    verify_sha256(archive, expected_sha256).map_err(ReplaceTargetError::integrity)?;
    if !sidecar_matches(&sidecar, expected_sha256) {
        return Err(ReplaceTargetError::integrity(format!(
            "Archiv-Pruefsumme {} stimmt nicht",
            sidecar.display()
        )));
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ReplaceTargetError> {
    let parent = path.parent().ok_or_else(|| {
        ReplaceTargetError::integrity("Pruefsummenpfad hat keinen Elternordner".to_string())
    })?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| ReplaceTargetError::io("Pruefsumme temporaer anlegen", error))?;
    temp.write_all(bytes)
        .map_err(|error| ReplaceTargetError::io("Pruefsumme schreiben", error))?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|error| ReplaceTargetError::io("Pruefsumme synchronisieren", error))?;
    temp.persist(path)
        .map(|_| ())
        .map_err(|error| ReplaceTargetError::io("Pruefsumme atomar einsetzen", error.error))
}

fn sidecar_matches(sidecar: &Path, expected_sha256: &str) -> bool {
    std::fs::read_to_string(sidecar)
        .ok()
        .and_then(|raw| raw.split_whitespace().next().map(str::to_ascii_lowercase))
        .is_some_and(|actual| actual == expected_sha256.to_ascii_lowercase())
}

fn sidecar_path(archive: &Path) -> PathBuf {
    let name = archive
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    archive.with_file_name(format!("{name}.sha256"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::sha256_file;

    #[test]
    fn archive_writes_hash_bound_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        let archive = dir.path().join("versions").join("app 1.0.0");
        std::fs::write(&target, b"current").unwrap();
        let hash = sha256_file(&target).unwrap();

        archive_current_app(&target, &hash, &archive).unwrap();

        assert_eq!(std::fs::read(&archive).unwrap(), b"current");
        assert!(sidecar_matches(&sidecar_path(&archive), &hash));
    }
}
