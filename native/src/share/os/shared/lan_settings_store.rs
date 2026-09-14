//! File store for `LanSettings` (`<app data>/lan_settings.json`).
use std::io;

use super::lan_settings::LanSettings;

const FILE_NAME: &str = "lan_settings.json";
const MAX_BYTES: u64 = 64 * 1024;

fn path() -> std::path::PathBuf {
    crate::support_dirs::app_data_file(FILE_NAME)
}

impl LanSettings {
    /// Missing file = defaults; an unreadable or malformed file is an error so
    /// callers never silently run with wrong settings.
    pub fn load() -> Result<Self, String> {
        let path = path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("LAN-Einstellungen lesen: {error}")),
        };
        if !metadata.file_type().is_file() {
            return Err("LAN-Einstellungen sind keine regulaere Datei".into());
        }
        if metadata.len() > MAX_BYTES {
            return Err("LAN-Einstellungen sind unplausibel gross".into());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("LAN-Einstellungen lesen: {error}"))?;
        Self::parse(&text)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("LAN-Einstellungen speichern: {error}"))?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, self.encode()?)
            .map_err(|error| format!("LAN-Einstellungen speichern: {error}"))?;
        std::fs::rename(&temporary, &path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("LAN-Einstellungen speichern: {error}")
        })
    }

    /// Load, apply `edit`, save; returns the committed settings.
    pub fn update(edit: impl FnOnce(&mut Self)) -> Result<Self, String> {
        let mut settings = Self::load()?;
        edit(&mut settings);
        settings.save()?;
        Ok(settings)
    }
}
