//! Folder activation is independent of the mounted-device preference.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OpenFoldersWith {
    #[default]
    App,
    FileManager,
}
