use super::hash::normalize_sha256;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const LAUNCH_COMPLETE_SUFFIX: &str = "launch-complete";
const LAUNCH_COMPLETE_SCHEMA: &str = "SMART-EXPLORER-LAUNCH-COMPLETE-V1";

pub(crate) fn launch_complete_path(
    last_applied: &Path,
    target_key: &str,
) -> Result<PathBuf, String> {
    let name = last_applied
        .file_name()
        .ok_or_else(|| {
            format!(
                "Update-Statuspfad {} hat keinen Dateinamen",
                last_applied.display()
            )
        })?
        .to_string_lossy();
    let target_id = normalize_sha256(target_key)?;
    Ok(last_applied.with_file_name(format!(".{name}.{LAUNCH_COMPLETE_SUFFIX}.{target_id}")))
}

pub(crate) fn launch_complete_matches(
    marker: &Path,
    target_key: &str,
    version: &str,
    installed_sha256: &str,
) -> Result<bool, String> {
    let metadata = match std::fs::symlink_metadata(marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Startabschluss {} pruefen: {error}",
                marker.display()
            ));
        }
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "Startabschluss {} ist keine regulaere Datei",
            marker.display()
        ));
    }
    let expected = launch_complete_payload(target_key, version, installed_sha256)?;
    if metadata.len() != (expected.len() + 33) as u64 {
        return Ok(false);
    }
    let mut raw = Vec::with_capacity(expected.len() + 33);
    std::fs::File::open(marker)
        .and_then(|file| {
            file.take((expected.len() + 34) as u64)
                .read_to_end(&mut raw)
        })
        .map_err(|error| format!("Startabschluss {} lesen: {error}", marker.display()))?;
    let Some(challenge) = raw.strip_prefix(expected.as_slice()) else {
        return Ok(false);
    };
    Ok(challenge.len() == 33
        && challenge[32] == b'\n'
        && challenge[..32].iter().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
pub(crate) fn write_launch_complete(
    marker: &Path,
    target_key: &str,
    version: &str,
    installed_sha256: &str,
) -> Result<(), String> {
    let mut record = launch_complete_payload(target_key, version, installed_sha256)?;
    record.extend_from_slice(format!("{}\n", "0".repeat(32)).as_bytes());
    atomic_write(marker, &record)
        .map_err(|error| format!("Startabschluss {} schreiben: {error}", marker.display()))
}

pub(crate) fn launch_complete_payload(
    target_key: &str,
    version: &str,
    installed_sha256: &str,
) -> Result<Vec<u8>, String> {
    if version.is_empty() || version.contains('\r') || version.contains('\n') {
        return Err("Startabschluss-Version ist ungueltig".to_string());
    }
    Ok(format!(
        "{LAUNCH_COMPLETE_SCHEMA}\n{}\n{version}\n{}\n",
        normalize_sha256(target_key)?,
        normalize_sha256(installed_sha256)?
    )
    .into_bytes())
}

struct HiddenFile {
    path: PathBuf,
    backup: Option<PathBuf>,
}

/// Makes updater-visible state consistent before the replacement app starts,
/// while retaining enough information to restore that state if launch fails.
pub(crate) struct PreparedBookkeeping {
    hidden: Vec<HiddenFile>,
    last_applied: PathBuf,
    version: Vec<u8>,
    finished: bool,
}

impl PreparedBookkeeping {
    pub(crate) fn prepare(
        last_applied: &Path,
        error_file: &Path,
        version: &str,
        hide_before_launch: &[&Path],
    ) -> Result<Self, String> {
        let mut paths = vec![last_applied, error_file];
        paths.extend_from_slice(hide_before_launch);
        validate_distinct_paths(&paths)?;

        let mut prepared = Self {
            hidden: Vec::with_capacity(paths.len()),
            last_applied: last_applied.to_path_buf(),
            version: version.as_bytes().to_vec(),
            finished: false,
        };
        for path in paths {
            if let Err(error) = prepared.hide(path) {
                return Err(prepared.fail_and_restore(error));
            }
        }
        if let Err(error) = atomic_write(last_applied, &prepared.version) {
            return Err(prepared.fail_and_restore(format!(
                "Update-Status {} schreiben: {error}",
                last_applied.display()
            )));
        }
        Ok(prepared)
    }

    pub(crate) fn rollback(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        let mut errors = Vec::new();
        for hidden in self.hidden.iter().rev() {
            match std::fs::symlink_metadata(&hidden.path) {
                Ok(metadata) if hidden.path == self.last_applied => {
                    if !metadata.file_type().is_file() {
                        errors.push(format!(
                            "neuer Update-Status {} ist keine regulaere Datei",
                            hidden.path.display()
                        ));
                        continue;
                    }
                    match std::fs::read(&hidden.path) {
                        Ok(raw) if raw == self.version => {
                            if let Err(error) = std::fs::remove_file(&hidden.path) {
                                errors.push(format!(
                                    "neuen Update-Status {} entfernen: {error}",
                                    hidden.path.display()
                                ));
                                continue;
                            }
                        }
                        Ok(_) => {
                            errors.push(format!(
                                "Update-Status {} wurde waehrend des Rollbacks veraendert",
                                hidden.path.display()
                            ));
                            continue;
                        }
                        Err(error) => {
                            errors.push(format!(
                                "neuen Update-Status {} lesen: {error}",
                                hidden.path.display()
                            ));
                            continue;
                        }
                    }
                }
                Ok(_) => {
                    errors.push(format!(
                        "Updater-Statuspfad {} wurde unerwartet neu angelegt",
                        hidden.path.display()
                    ));
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    errors.push(format!(
                        "Updater-Statuspfad {} pruefen: {error}",
                        hidden.path.display()
                    ));
                    continue;
                }
            }

            if let Some(backup) = &hidden.backup {
                if let Err(error) = std::fs::rename(backup, &hidden.path) {
                    errors.push(format!(
                        "Updater-Status {} wiederherstellen: {error}",
                        hidden.path.display()
                    ));
                    continue;
                }
            }
            if let Err(error) = sync_parent(&hidden.path) {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            self.finished = true;
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Commit only removes private backups. Public status paths were already
    /// made consistent before launch, so this cannot race the new app's reads.
    pub(crate) fn commit(&mut self) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        let mut warnings = Vec::new();
        for hidden in &self.hidden {
            if let Some(backup) = &hidden.backup {
                if let Err(error) = std::fs::remove_file(backup) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        warnings.push(format!(
                            "Updater-Statussicherung {} entfernen: {error}",
                            backup.display()
                        ));
                    }
                } else if let Err(error) = sync_parent(backup) {
                    warnings.push(error);
                }
            }
        }
        self.finished = true;
        warnings
    }

    fn hide(&mut self, path: &Path) -> Result<(), String> {
        if path.as_os_str().is_empty() {
            return Err("Updater-Statuspfad ist leer".to_string());
        }
        let backup = match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    return Err(format!(
                        "Updater-Statuspfad {} ist keine regulaere Datei",
                        path.display()
                    ));
                }
                let backup = unique_sibling(path, "update-status-backup")?;
                std::fs::rename(path, &backup).map_err(|error| {
                    format!("Updater-Status {} sichern: {error}", path.display())
                })?;
                Some(backup)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "Updater-Status {} pruefen: {error}",
                    path.display()
                ));
            }
        };
        self.hidden.push(HiddenFile {
            path: path.to_path_buf(),
            backup,
        });
        if self
            .hidden
            .last()
            .is_some_and(|hidden| hidden.backup.is_some())
        {
            sync_parent(path)?;
        }
        Ok(())
    }

    fn fail_and_restore(&mut self, primary: String) -> String {
        match self.rollback() {
            Ok(()) => primary,
            Err(rollback) => format!("{primary}; Status-Rollback fehlgeschlagen: {rollback}"),
        }
    }
}

impl Drop for PreparedBookkeeping {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.rollback();
        }
    }
}

fn validate_distinct_paths(paths: &[&Path]) -> Result<(), String> {
    for (index, path) in paths.iter().enumerate() {
        if paths[..index].contains(path) {
            return Err(format!(
                "Updater-Statuspfad {} wurde mehrfach angegeben",
                path.display()
            ));
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Ordner {} anlegen: {error}", parent.display()))?;
    }
    let pending = unique_sibling(path, "update-status-pending")?;
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)
            .map_err(|error| format!("Temp-Datei {} anlegen: {error}", pending.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("Temp-Datei {} schreiben: {error}", pending.display()))?;
        file.sync_all().map_err(|error| {
            format!("Temp-Datei {} synchronisieren: {error}", pending.display())
        })?;
        drop(file);
        std::fs::rename(&pending, path)
            .map_err(|error| format!("Status {} einsetzen: {error}", path.display()))?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Statusordner {} synchronisieren: {error}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn unique_sibling(path: &Path, role: &str) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("Statuspfad {} hat keinen Dateinamen", path.display()))?
        .to_string_lossy();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for nonce in 0..100u32 {
        let candidate = path.with_file_name(format!(
            "{name}.{role}.{}.{}.{}",
            std::process::id(),
            nanos,
            nonce
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Kein freier temporaerer Statuspfad neben {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_restores_previous_status_error_and_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let last = dir.path().join("last.txt");
        let error = dir.path().join("error.txt");
        let manifest = dir.path().join("manifest.json");
        std::fs::write(&last, b"0.5.119").unwrap();
        std::fs::write(&error, b"old error").unwrap();
        std::fs::write(&manifest, b"old manifest").unwrap();

        let mut prepared =
            PreparedBookkeeping::prepare(&last, &error, "0.5.121", &[&manifest]).unwrap();
        assert_eq!(std::fs::read(&last).unwrap(), b"0.5.121");
        assert!(!error.exists());
        assert!(!manifest.exists());

        prepared.rollback().unwrap();
        assert_eq!(std::fs::read(last).unwrap(), b"0.5.119");
        assert_eq!(std::fs::read(error).unwrap(), b"old error");
        assert_eq!(std::fs::read(manifest).unwrap(), b"old manifest");
    }

    #[test]
    fn commit_leaves_new_status_and_hidden_files_absent() {
        let dir = tempfile::tempdir().unwrap();
        let last = dir.path().join("last.txt");
        let error = dir.path().join("error.txt");
        let manifest = dir.path().join("manifest.json");
        std::fs::write(&error, b"stale").unwrap();
        std::fs::write(&manifest, b"staged").unwrap();

        let mut prepared =
            PreparedBookkeeping::prepare(&last, &error, "0.5.121", &[&manifest]).unwrap();
        assert!(prepared.commit().is_empty());

        assert_eq!(std::fs::read(last).unwrap(), b"0.5.121");
        assert!(!error.exists());
        assert!(!manifest.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn changed_new_status_is_not_clobbered_during_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let last = dir.path().join("last.txt");
        let error = dir.path().join("error.txt");
        std::fs::write(&last, b"0.5.119").unwrap();
        let mut prepared = PreparedBookkeeping::prepare(&last, &error, "0.5.121", &[]).unwrap();
        std::fs::write(&last, b"concurrent writer").unwrap();

        assert!(prepared.rollback().is_err());
        assert_eq!(std::fs::read(last).unwrap(), b"concurrent writer");
    }

    #[test]
    fn launch_complete_record_binds_version_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let last = dir.path().join("last.txt");
        let target = dir.path().join("app");
        std::fs::write(&target, b"app").unwrap();
        let target_key = crate::instance::target_key(&target);
        let marker = launch_complete_path(&last, &target_key).unwrap();
        let sha256 = "a".repeat(64);

        write_launch_complete(&marker, &target_key, "0.5.121", &sha256).unwrap();

        assert!(launch_complete_matches(&marker, &target_key, "0.5.121", &sha256).unwrap());
        assert!(!launch_complete_matches(&marker, &target_key, "0.5.120", &sha256).unwrap());
        assert!(
            !launch_complete_matches(&marker, &target_key, "0.5.121", &"b".repeat(64)).unwrap()
        );
    }

    #[test]
    fn malformed_launch_complete_record_is_not_proof() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");
        let target = dir.path().join("app");
        std::fs::write(&target, b"app").unwrap();
        std::fs::write(&marker, b"bad record\n").unwrap();
        let target_key = crate::instance::target_key(&target);

        assert!(
            !launch_complete_matches(&marker, &target_key, "0.5.121", &"a".repeat(64)).unwrap()
        );
    }

    #[test]
    fn app_written_challenged_receipt_is_completion_proof() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");
        let target = dir.path().join("app");
        std::fs::write(&target, b"app").unwrap();
        let target_key = crate::instance::target_key(&target);
        let sha256 = "a".repeat(64);
        let mut record = launch_complete_payload(&target_key, "0.5.121", &sha256).unwrap();
        record.extend_from_slice(format!("{}\n", "b".repeat(32)).as_bytes());
        std::fs::write(&marker, record).unwrap();

        assert!(launch_complete_matches(&marker, &target_key, "0.5.121", &sha256).unwrap());
    }
}
