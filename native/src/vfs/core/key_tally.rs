//! What a keys-only walk (`KeyedWalk::KeysOnly`) counts for the subtree of
//! each folder next to its key, so a caller that finds two folders equal by
//! key can account for everything in them without listing either side
//! (`transfer::engine::update_keys`).
use serde::{Deserialize, Serialize};

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// The subtree of one folder, the folder itself included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTally {
    /// Regular files.
    #[serde(default)]
    pub files: u64,
    /// Their sizes added.
    #[serde(default)]
    pub bytes: u64,
    /// Entries a copy leaves out and reports: links, special files, omissions
    /// with their reason, a folder that could not be read, and names no
    /// transfer takes (a backslash, a NUL, the app trash folder).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted: u64,
    /// Regular files with an unknown modification time (0): the file rule
    /// keeps both sides of such a file, so equal keys do not settle it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub untimed: u64,
    /// Folder levels below this folder (0 = no subfolder).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub height: u32,
}

impl KeyTally {
    /// Nothing in the subtree needs a look of its own: no omission, no file
    /// of unknown time.
    pub fn clean(&self) -> bool {
        self.omitted == 0 && self.untimed == 0
    }

    /// Adds a subfolder's subtree (one level deeper).
    pub fn add_child(&mut self, child: &KeyTally) {
        self.files = self.files.saturating_add(child.files);
        self.bytes = self.bytes.saturating_add(child.bytes);
        self.omitted = self.omitted.saturating_add(child.omitted);
        self.untimed = self.untimed.saturating_add(child.untimed);
        self.height = self.height.max(child.height.saturating_add(1));
    }
}

/// Whether a name is one no transfer takes as it is: it is counted as an
/// omission, so the subtree is walked by the transfer itself and reports it.
pub fn name_is_unusual(name: &str) -> bool {
    name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\0'])
        || name == crate::apptrash::TRASH_DIR_NAME
}
