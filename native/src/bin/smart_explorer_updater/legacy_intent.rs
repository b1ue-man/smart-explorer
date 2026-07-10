use super::hash::normalize_sha256;
use super::instance;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const INTENT_SCHEMA: &str = "SMART-EXPLORER-LEGACY-UPDATE-INTENT-V1";
const MAX_INTENT_BYTES: u64 = 320;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyIntent {
    path: PathBuf,
    target_key: String,
    version: String,
    previous_sha256: String,
    staged_sha256: String,
}

impl LegacyIntent {
    pub(crate) fn path_for(target: &Path) -> PathBuf {
        let target_key = instance::target_key(target);
        super::logging::appdata_dir().join(format!("legacy-update-intent-{target_key}"))
    }

    pub(crate) fn load(target: &Path) -> Result<Option<Self>, String> {
        let path = Self::path_for(target);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "Legacy-Update-Absicht {} pruefen: {error}",
                    path.display()
                ));
            }
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_INTENT_BYTES {
            return Err(format!(
                "Legacy-Update-Absicht {} ist keine kleine regulaere Datei",
                path.display()
            ));
        }
        let mut raw = Vec::with_capacity(metadata.len() as usize);
        std::fs::File::open(&path)
            .and_then(|file| file.take(MAX_INTENT_BYTES + 1).read_to_end(&mut raw))
            .map_err(|error| format!("Legacy-Update-Absicht {} lesen: {error}", path.display()))?;
        let intent = Self::parse(path, &raw)?;
        let expected_key = instance::target_key(target);
        if intent.target_key != expected_key {
            return Err("Legacy-Update-Absicht ist an ein anderes Programmziel gebunden".into());
        }
        Ok(Some(intent))
    }

    pub(crate) fn create(
        target: &Path,
        version: &str,
        previous_sha256: &str,
        staged_sha256: &str,
    ) -> Result<Self, String> {
        let intent = Self {
            path: Self::path_for(target),
            target_key: normalize_sha256(&instance::target_key(target))?,
            version: validate_version(version)?,
            previous_sha256: normalize_sha256(previous_sha256)?,
            staged_sha256: normalize_sha256(staged_sha256)?,
        };
        let payload = intent.payload();
        publish_new(&intent.path, &payload).map_err(|error| {
            format!(
                "Legacy-Update-Absicht {} dauerhaft schreiben: {error}",
                intent.path.display()
            )
        })?;
        Ok(intent)
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        match Self::load_from_path(&self.path)? {
            None => return Ok(()),
            Some(current) if current == *self => {}
            Some(_) => {
                return Err(format!(
                    "Legacy-Update-Absicht {} wurde unerwartet veraendert",
                    self.path.display()
                ));
            }
        }
        std::fs::remove_file(&self.path).map_err(|error| {
            format!(
                "Legacy-Update-Absicht {} entfernen: {error}",
                self.path.display()
            )
        })?;
        sync_parent(&self.path).map_err(|error| {
            format!(
                "Legacy-Update-Absichtsordner {} synchronisieren: {error}",
                self.path.display()
            )
        })
    }

    pub(crate) fn target_key(&self) -> &str {
        &self.target_key
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn previous_sha256(&self) -> &str {
        &self.previous_sha256
    }

    pub(crate) fn staged_sha256(&self) -> &str {
        &self.staged_sha256
    }

    fn load_from_path(path: &Path) -> Result<Option<Self>, String> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Absicht {} pruefen: {error}", path.display())),
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_INTENT_BYTES {
            return Err(format!("Absicht {} ist ungueltig", path.display()));
        }
        let mut raw = Vec::with_capacity(metadata.len() as usize);
        std::fs::File::open(path)
            .and_then(|file| file.take(MAX_INTENT_BYTES + 1).read_to_end(&mut raw))
            .map_err(|error| format!("Absicht {} lesen: {error}", path.display()))?;
        Self::parse(path.to_path_buf(), &raw).map(Some)
    }

    fn parse(path: PathBuf, raw: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(raw)
            .map_err(|error| format!("Legacy-Update-Absicht ist kein UTF-8: {error}"))?;
        let mut lines = text.split('\n');
        let schema = lines.next();
        let target_key = lines.next();
        let version = lines.next();
        let previous_sha256 = lines.next();
        let staged_sha256 = lines.next();
        if schema != Some(INTENT_SCHEMA) || lines.next() != Some("") || lines.next().is_some() {
            return Err("Legacy-Update-Absicht hat ein ungueltiges Format".into());
        }
        Ok(Self {
            path,
            target_key: normalize_sha256(target_key.unwrap_or_default())?,
            version: validate_version(version.unwrap_or_default())?,
            previous_sha256: normalize_sha256(previous_sha256.unwrap_or_default())?,
            staged_sha256: normalize_sha256(staged_sha256.unwrap_or_default())?,
        })
    }

    fn payload(&self) -> Vec<u8> {
        format!(
            "{INTENT_SCHEMA}\n{}\n{}\n{}\n{}\n",
            self.target_key, self.version, self.previous_sha256, self.staged_sha256
        )
        .into_bytes()
    }
}

fn validate_version(version: &str) -> Result<String, String> {
    if version.is_empty()
        || version.len() > 64
        || !version.is_ascii()
        || version.contains(['\r', '\n'])
    {
        Err("Legacy-Update-Absicht enthaelt eine ungueltige Version".into())
    } else {
        Ok(version.to_string())
    }
}

fn publish_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pending = path.with_file_name(format!(
        ".legacy-update-intent-pending-{}-{}",
        std::process::id(),
        nonce()
    ));
    let result = (|| {
        let mut file = create_private_file(&pending)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        publish_no_replace(&pending, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(windows)]
fn publish_no_replace(pending: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let pending = pending
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(pending.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn publish_no_replace(pending: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::hard_link(pending, target)?;
    if let Err(error) = sync_parent(target) {
        let _ = std::fs::remove_file(target);
        let _ = std::fs::remove_file(pending);
        return Err(error);
    }
    std::fs::remove_file(pending)?;
    sync_parent(target)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn publish_no_replace(_pending: &Path, _target: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "legacy update intents are unsupported on this operating system",
    ))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_is_target_bound_exclusive_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        std::fs::write(&target, b"old").unwrap();
        let previous = super::super::hash::sha256_file(&target).unwrap();
        let staged = "a".repeat(64);
        let intent = LegacyIntent::create(&target, "0.5.121", &previous, &staged).unwrap();

        assert_eq!(LegacyIntent::load(&target).unwrap(), Some(intent.clone()));
        assert!(LegacyIntent::create(&target, "0.5.121", &previous, &staged).is_err());
        intent.clear().unwrap();
        assert!(LegacyIntent::load(&target).unwrap().is_none());
    }

    #[test]
    fn malformed_or_tampered_intent_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        std::fs::write(&target, b"old").unwrap();
        let path = LegacyIntent::path_for(&target);
        std::fs::write(&path, b"bad\n").unwrap();

        assert!(LegacyIntent::load(&target).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
