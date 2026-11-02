//! F2 (plan 2026-10-31): the Direct folders of one device. A Direct grant
//! with an entry sees exactly the selected folders, in the actual file
//! access check (`SessionAuthorization::direct`), not only in a view.
use serde::{Deserialize, Serialize};

use super::ShareExportConfig;
use crate::share::connection_rights::FolderChoice;

/// The folders one Direct device sees, bound to its exact key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFolders {
    pub device_id: String,
    pub public_key: String,
    /// `SharedRoot::path` of each visible Direct export.
    #[serde(default)]
    pub paths: Vec<String>,
    /// The exported saved connections (`/Verbindungen`) are visible too.
    #[serde(default)]
    pub connections: bool,
}

impl ShareExportConfig {
    /// The selection recorded for `device_id`, whichever key it names.
    pub fn device_folders_of(&self, device_id: &str) -> Option<&DeviceFolders> {
        self.device_folders
            .iter()
            .find(|entry| entry.device_id == device_id)
    }

    /// What the Direct device `(device_id, public_key)` sees. Without an
    /// entry: every export (grants persisted before F2). With an entry: only
    /// its folders and, when chosen, the saved connections. An entry of the
    /// same device id for another key shows nothing (fail closed). The
    /// result carries no selections of other devices.
    pub fn visible_to(&self, device_id: &str, public_key: &str) -> ShareExportConfig {
        let Some(entry) = self.device_folders_of(device_id) else {
            return ShareExportConfig {
                device_folders: Vec::new(),
                ..self.clone()
            };
        };
        if entry.public_key != public_key {
            return ShareExportConfig::default();
        }
        ShareExportConfig {
            roots: self
                .roots
                .iter()
                .filter(|root| entry.paths.contains(&root.path))
                .cloned()
                .collect(),
            include_connections: entry.connections && self.include_connections,
            shared_connections: if entry.connections {
                self.shared_connections.clone()
            } else {
                Vec::new()
            },
            device_folders: Vec::new(),
        }
    }

    /// The folder choice the device currently has (for the menu).
    pub fn folder_choice_of(&self, device_id: &str, public_key: &str) -> FolderChoice {
        match self.device_folders_of(device_id) {
            None => FolderChoice::All,
            Some(entry) if entry.public_key != public_key => FolderChoice::None,
            Some(entry) if entry.paths.is_empty() && !entry.connections => FolderChoice::None,
            Some(entry) => FolderChoice::Selected {
                paths: entry.paths.clone(),
                connections: entry.connections,
            },
        }
    }

    /// Records `choice` for the device. `All` removes the entry (every
    /// export, also later ones); `Selected` must name Direct exports.
    /// Returns whether anything changed.
    pub fn set_device_folders(
        &mut self,
        device_id: &str,
        public_key: &str,
        choice: &FolderChoice,
    ) -> Result<bool, String> {
        let entry = match choice {
            FolderChoice::All => None,
            FolderChoice::None => Some(DeviceFolders {
                device_id: device_id.into(),
                public_key: public_key.into(),
                paths: Vec::new(),
                connections: false,
            }),
            FolderChoice::Selected { paths, connections } => {
                if let Some(missing) = paths
                    .iter()
                    .find(|path| !self.roots.iter().any(|root| &root.path == *path))
                {
                    return Err(format!(
                        "Ordner ist keine Direkt-Freigabe: {missing}; zuerst mit `se share export add` freigeben"
                    ));
                }
                Some(DeviceFolders {
                    device_id: device_id.into(),
                    public_key: public_key.into(),
                    paths: paths.clone(),
                    connections: *connections,
                })
            }
        };
        let before = self.device_folders.clone();
        self.device_folders
            .retain(|existing| existing.device_id != device_id);
        if let Some(entry) = entry {
            self.device_folders.push(entry);
        }
        Ok(before != self.device_folders)
    }

    /// Drops the selections of devices `keep` does not name.
    pub fn retain_device_folders(&mut self, keep: impl Fn(&DeviceFolders) -> bool) -> bool {
        let before = self.device_folders.len();
        self.device_folders.retain(|entry| keep(entry));
        before != self.device_folders.len()
    }
}
