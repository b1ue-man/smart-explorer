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
        let text = crate::support_dirs::read_private_text(&path, MAX_BYTES)
            .map_err(|error| format!("Uplink-Status lesen: {error}"))?;
        Self::parse(&text)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path();
        crate::support_dirs::write_private_atomic(&path, self.encode()?.as_bytes())
            .map_err(|error| format!("Uplink-Status speichern: {error}"))
    }
}
