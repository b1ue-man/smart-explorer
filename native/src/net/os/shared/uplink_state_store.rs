//! File store for `UplinkState` (`<app data>/lan_uplink_state.json`).
use std::io;

use crate::net::UplinkState;

const FILE_NAME: &str = "lan_uplink_state.json";
const MAX_BYTES: u64 = 64 * 1024;

fn path() -> std::path::PathBuf {
    crate::support_dirs::app_data_file(FILE_NAME)
}

impl UplinkState {
    pub fn load() -> Result<Self, String> {
        let path = path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("Uplink-Status lesen: {error}")),
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_BYTES {
            return Err("Uplink-Status-Datei ist ungueltig".into());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("Uplink-Status lesen: {error}"))?;
        Self::parse(&text)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Uplink-Status speichern: {error}"))?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, self.encode()?)
            .map_err(|error| format!("Uplink-Status speichern: {error}"))?;
        std::fs::rename(&temporary, &path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("Uplink-Status speichern: {error}")
        })
    }
}
