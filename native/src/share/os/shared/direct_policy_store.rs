//! File store for the Direct request policy (`<app data>/share_direct_policy.json`,
//! FC5): whether requests carrying this device's Direct code from identities
//! without a grant are accepted automatically. The file is a device preference
//! like the LAN settings; the profile transactions only read it.
use std::io;

use serde::{Deserialize, Serialize};

use super::direct_relation::DirectRequestPolicy;

const FILE_NAME: &str = "share_direct_policy.json";
const MAX_BYTES: u64 = 16 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct PolicyFile {
    #[serde(default)]
    requests: DirectRequestPolicy,
}

fn path() -> std::path::PathBuf {
    crate::support_dirs::app_data_file(FILE_NAME)
}

impl DirectRequestPolicy {
    /// Missing file = `Ask`. An unreadable or malformed file is an error; the
    /// daemon then decides as with `Ask` (never more access than chosen).
    pub fn load() -> Result<Self, String> {
        let path = path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("Direkt-Richtlinie lesen: {error}")),
        };
        if !metadata.file_type().is_file() {
            return Err("Direkt-Richtlinie ist keine regulaere Datei".into());
        }
        if metadata.len() > MAX_BYTES {
            return Err("Direkt-Richtlinie ist unplausibel gross".into());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("Direkt-Richtlinie lesen: {error}"))?;
        let file: PolicyFile = serde_json::from_str(&text)
            .map_err(|error| format!("Direkt-Richtlinie ist beschaedigt: {error}"))?;
        Ok(file.requests)
    }

    /// The policy for a decision right now; an unreadable file counts as `Ask`.
    pub fn current() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn save(self) -> Result<(), String> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Direkt-Richtlinie speichern: {error}"))?;
        }
        let text = serde_json::to_string_pretty(&PolicyFile { requests: self })
            .map_err(|error| format!("Direkt-Richtlinie kodieren: {error}"))?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)
            .map_err(|error| format!("Direkt-Richtlinie speichern: {error}"))?;
        std::fs::rename(&temporary, &path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("Direkt-Richtlinie speichern: {error}")
        })
    }
}
