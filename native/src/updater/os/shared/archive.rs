use std::io::Write;
use std::path::{Path, PathBuf};

use super::config::appdata_dir;
use super::core::{parse_sha256_file, parse_ver, sha256_file};
use super::os;
use super::staging::cleanup_abandoned_staging;
use super::types::VerifiedPayload;

/// Filename prefix for the renamed-out running binary (`<stem>_old`).
pub(super) fn old_binary_prefix(cur_exe: &Path) -> String {
    let stem = cur_exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "smart_explorer".into());
    format!("{}_old", stem)
}

/// Delete leftovers from previous updates (best effort, with retries since an
/// old process may still hold one).
pub fn cleanup_old_binaries() {
    std::thread::Builder::new()
        .name("update-cleanup".into())
        .spawn(|| {
            cleanup_abandoned_staging();
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(_) => return,
            };
            let dir = match exe.parent() {
                Some(d) => d.to_path_buf(),
                None => return,
            };
            let prefix = old_binary_prefix(&exe);
            for _ in 0..10 {
                let mut any_left = false;
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with(&prefix)
                            && name.ends_with(os::binary_suffix())
                            && std::fs::remove_file(e.path()).is_err()
                        {
                            any_left = true;
                        }
                    }
                }
                if !any_left {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        })
        .ok();
}

pub(super) fn versions_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|d| d.join("versions"))
}

pub(super) fn pin_path() -> PathBuf {
    appdata_dir().join("update_pinned.txt")
}

/// Auto-update on launch is paused (the user reverted to an older version).
pub fn is_auto_update_paused() -> bool {
    pin_path().exists()
}

/// The version we're pinned to, if any.
pub fn pinned_version() -> Option<String> {
    pinned_version_checked().ok().flatten()
}

pub(super) fn pinned_version_checked() -> Result<Option<String>, String> {
    let path = pin_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Update-Pin {} lesen: {error}", path.display())),
    };
    let version = raw.trim();
    if version.is_empty()
        || !version
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '-' || ch.is_ascii_alphabetic())
    {
        return Err(format!(
            "Update-Pin {} ist beschaedigt: {:?}",
            path.display(),
            raw
        ));
    }
    Ok(Some(version.to_string()))
}

pub(super) fn set_pin(version: &str) -> std::io::Result<()> {
    if version.trim().is_empty()
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Ungueltige Rollback-Version",
        ));
    }
    let path = pin_path();
    write_atomic(&path, version.as_bytes())
}

/// Resume automatic updates (clears the rollback pin).
pub fn resume_auto_update() -> std::io::Result<()> {
    match std::fs::remove_file(pin_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) fn restore_pin(previous: Option<&str>) -> std::io::Result<()> {
    match previous {
        Some(version) => set_pin(version),
        None => resume_auto_update(),
    }
}

pub(super) fn exe_stem(cur_exe: &Path) -> String {
    cur_exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Smart Explorer".into())
}

/// Copy the currently-running binary into the versions archive. The payload
/// and its SHA-256 sidecar are written through same-directory temporary files;
/// callers must not proceed with a destructive replacement if this fails.
pub(super) fn archive_binary(version: &str) -> Result<VerifiedPayload, String> {
    let vd = versions_dir().ok_or_else(|| "Versionsordner unbekannt".to_string())?;
    std::fs::create_dir_all(&vd)
        .map_err(|error| format!("Versionsordner {} anlegen: {error}", vd.display()))?;
    let cur =
        std::env::current_exe().map_err(|error| format!("Eigener Pfad unbekannt: {error}"))?;
    let dest = vd.join(format!(
        "{} {}{}",
        exe_stem(&cur),
        version,
        os::binary_suffix()
    ));
    if dest.exists() {
        if let Ok(hash) = archived_sha256(&dest) {
            return VerifiedPayload::new(dest, hash);
        }
    }

    let mut temp = tempfile::NamedTempFile::new_in(&vd)
        .map_err(|error| format!("Archiv temporaer anlegen: {error}"))?;
    let mut source = std::fs::File::open(&cur)
        .map_err(|error| format!("Programmdatei {} lesen: {error}", cur.display()))?;
    std::io::copy(&mut source, temp.as_file_mut())
        .map_err(|error| format!("Programmdatei archivieren: {error}"))?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|error| format!("Archiv synchronisieren: {error}"))?;
    let hash = sha256_file(temp.path())?;
    temp.persist(&dest)
        .map_err(|error| format!("Archiv atomar einsetzen: {}", error.error))?;
    write_atomic(&archive_sidecar(&dest), format!("{hash}\n").as_bytes())
        .map_err(|error| format!("Archiv-Pruefsumme schreiben: {error}"))?;
    archived_sha256(&dest)?;
    VerifiedPayload::new(dest, hash)
}

/// Preserve the currently-running binary in the versions archive so it can be
/// rolled back to after a future update.
pub fn archive_current_version() {
    std::thread::Builder::new()
        .name("version-archive".into())
        .spawn(|| {
            if let Err(error) = archive_binary(env!("CARGO_PKG_VERSION")) {
                eprintln!("Smart Explorer konnte die aktuelle Version nicht archivieren: {error}");
            }
        })
        .ok();
}

pub(super) fn archived_sha256(archive: &Path) -> Result<String, String> {
    let sidecar = archive_sidecar(archive);
    let raw = std::fs::read_to_string(&sidecar)
        .map_err(|error| format!("Archiv-Pruefsumme {} lesen: {error}", sidecar.display()))?;
    let name = sidecar
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Archiv-Pruefsumme");
    let expected = parse_sha256_file(&raw, name)?;
    let actual = sha256_file(archive)?;
    if actual.eq_ignore_ascii_case(&expected) {
        Ok(expected)
    } else {
        Err(format!(
            "Archiv {} ist beschaedigt: erwartet {}, erhalten {}",
            archive.display(),
            expected,
            actual
        ))
    }
}

pub(super) fn archive_sidecar(archive: &Path) -> PathBuf {
    let name = archive
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    archive.with_file_name(format!("{name}.sha256"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Pfad ohne Elternordner")
    })?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map(|_| ()).map_err(|error| error.error)
}

/// Archived versions available to roll back to, newest first.
pub fn list_archived_versions() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    if let Some(vd) = versions_dir() {
        if let Ok(rd) = std::fs::read_dir(&vd) {
            for e in rd.flatten() {
                let p = e.path();
                if !os::is_archived_binary(&p) {
                    continue;
                }
                if let Some(name) = os::archived_name_without_binary_suffix(&p) {
                    if let Some(ver) = name.rsplit(' ').next() {
                        if ver
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                        {
                            out.push((ver.to_string(), p.clone()));
                        }
                    }
                }
            }
        }
    }
    out.sort_by_key(|entry| std::cmp::Reverse(parse_ver(&entry.0)));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}
