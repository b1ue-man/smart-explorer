use std::io::Write;
use std::path::Path;

use super::config::{appdata_dir, staged_update_manifest_path};
use super::core::{staged_sha256_from_path, verify_sha256};
use super::feed::Feed;
use super::types::{StagedUpdate, VerifiedPayload};

const STAGED_PREFIXES: [&str; 3] = ["update_download_", "updater_download_", "cli_download_"];

pub(super) struct StagingGuard {
    paths: Vec<std::path::PathBuf>,
    armed: bool,
}

impl StagingGuard {
    pub(super) fn new() -> Self {
        Self {
            paths: Vec::new(),
            armed: true,
        }
    }

    pub(super) fn track(&mut self, payload: &VerifiedPayload) {
        self.paths.push(payload.path().to_path_buf());
    }

    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            for path in &self.paths {
                remove_staged_path(path);
            }
        }
    }
}

pub(super) fn stage_from_feed(feed: &Feed, version: &str) -> Result<StagedUpdate, String> {
    let mut guard = StagingGuard::new();
    let app = feed.fetch_exe(version)?;
    guard.track(&app);
    let helper = feed.fetch_updater_exe(version)?;
    guard.track(&helper);
    let cli = feed.fetch_cli_exe(version)?;
    guard.track(&cli);
    let bundle = StagedUpdate::new(version.to_string(), app, helper, cli)?;
    persist_staged_update(&bundle)?;
    guard.disarm();
    Ok(bundle)
}

pub fn load_staged_update() -> Result<Option<StagedUpdate>, String> {
    let path = staged_update_manifest_path();
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Staging-Manifest {} lesen: {error}",
                path.display()
            ));
        }
    };
    let bundle: StagedUpdate = serde_json::from_slice(&raw).map_err(|error| {
        format!(
            "Staging-Manifest {} ist beschaedigt: {error}",
            path.display()
        )
    })?;
    verify_staged_update(&bundle)?;
    Ok(Some(bundle))
}

pub(super) fn persist_staged_update(bundle: &StagedUpdate) -> Result<(), String> {
    verify_staged_update(bundle)?;
    let previous = load_staged_update().ok().flatten();
    let path = staged_update_manifest_path();
    let parent = path
        .parent()
        .ok_or_else(|| "Staging-Manifest hat keinen Elternordner".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Staging-Ordner {} anlegen: {error}", parent.display()))?;
    let raw = serde_json::to_vec(bundle)
        .map_err(|error| format!("Staging-Manifest serialisieren: {error}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Staging-Manifest temporaer anlegen: {error}"))?;
    temp.write_all(&raw)
        .map_err(|error| format!("Staging-Manifest schreiben: {error}"))?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|error| format!("Staging-Manifest synchronisieren: {error}"))?;
    temp.persist(&path)
        .map_err(|error| format!("Staging-Manifest atomar einsetzen: {}", error.error))?;

    if let Some(previous) = previous {
        for payload in previous.payloads() {
            if !bundle
                .payloads()
                .iter()
                .any(|current| current.path() == payload.path())
            {
                remove_staged_path(payload.path());
            }
        }
    }
    Ok(())
}

pub fn verify_staged_update(bundle: &StagedUpdate) -> Result<(), String> {
    bundle.validate_schema()?;
    for (label, payload) in [
        ("App", bundle.app()),
        ("Updater", bundle.helper()),
        ("CLI", bundle.cli()),
    ] {
        validate_payload_path(payload)?;
        verify_sha256(payload.path(), payload.sha256())
            .map_err(|error| format!("{label}-Payload: {error}"))?;
    }
    Ok(())
}

pub(super) fn manifest_matches(bundle: &StagedUpdate) -> Result<(), String> {
    match load_staged_update()? {
        Some(current) if current == *bundle => Ok(()),
        Some(_) => Err("Ein neueres gestagtes Update hat dieses Update ersetzt".to_string()),
        None => Err("Das Staging-Manifest fuer dieses Update fehlt".to_string()),
    }
}

pub fn discard_staged_update(bundle: &StagedUpdate) -> Result<(), String> {
    manifest_matches(bundle)?;
    remove_manifest()?;
    for payload in bundle.payloads() {
        remove_staged_path(payload.path());
    }
    Ok(())
}

pub(super) fn remove_manifest() -> Result<(), String> {
    let path = staged_update_manifest_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Staging-Manifest {} entfernen: {error}",
            path.display()
        )),
    }
}

pub(super) fn manifest_path() -> std::path::PathBuf {
    staged_update_manifest_path()
}

pub(super) fn cleanup_abandoned_staging() {
    let retained = match load_staged_update() {
        Ok(Some(bundle)) => bundle
            .payloads()
            .into_iter()
            .map(|payload| payload.path().to_path_buf())
            .collect::<Vec<_>>(),
        Ok(None) => Vec::new(),
        Err(_) => return,
    };
    let Ok(entries) = std::fs::read_dir(appdata_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if retained.iter().any(|keep| keep == &path) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if STAGED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
            && staged_sha256_from_path(&path).is_some()
        {
            remove_staged_path(&path);
        }
    }
}

fn validate_payload_path(payload: &VerifiedPayload) -> Result<(), String> {
    let parent = payload
        .path()
        .parent()
        .ok_or_else(|| "Gestagter Payload hat keinen Elternordner".to_string())?;
    if parent != appdata_dir() {
        return Err(format!(
            "Gestagter Payload liegt ausserhalb des Update-Ordners: {}",
            payload.path().display()
        ));
    }
    let name = payload
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Gestagter Payload hat keinen gueltigen Dateinamen".to_string())?;
    if !STAGED_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return Err(format!("Unbekannter Staging-Dateiname: {name}"));
    }
    match staged_sha256_from_path(payload.path()) {
        Some(hash) if hash.eq_ignore_ascii_case(payload.sha256()) => Ok(()),
        _ => Err(format!(
            "Staging-Dateiname und SHA-256 stimmen nicht ueberein: {}",
            payload.path().display()
        )),
    }
}

fn remove_staged_path(path: &Path) {
    if path.parent() == Some(appdata_dir().as_path()) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_manifest_payload_outside_appdata() {
        let hash = "a".repeat(64);
        let payload = VerifiedPayload::new(std::env::temp_dir().join("outside"), hash).unwrap();
        assert!(validate_payload_path(&payload).is_err());
    }
}
