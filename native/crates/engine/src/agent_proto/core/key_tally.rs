//! What a keys-only walk (`KeyedWalk::KeysOnly`) counts for the subtree of
//! each folder next to its key, so a caller that finds two folders equal by
//! key can account for everything in them without listing either side
//! (`transfer::engine::update_keys`).
//!
//! One definition for the app (`vfs::KeyTally`, which adds the serde form the
//! Share wire uses) and for the standalone `se-agent` (which has no serde):
//! this file needs nothing but `std`.

/// The name of the app's trash folder (`apptrash::TRASH_DIR_NAME`; the app
/// asserts the two are equal, `vfs/core/key_tally.rs`). The agent has no
/// `apptrash` module, and a transfer takes no folder of this name.
pub const APP_TRASH_DIR_NAME: &str = ".SmartExplorer-Papierkorb";

/// The subtree of one folder, the folder itself included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyTally {
    /// Regular files.
    pub files: u64,
    /// Their sizes added.
    pub bytes: u64,
    /// Entries a copy leaves out and reports: links, special files, omissions
    /// with their reason, a folder that could not be read, and names no
    /// transfer takes (a backslash, a NUL, the app trash folder).
    pub omitted: u64,
    /// Regular files with an unknown modification time (0): the file rule
    /// keeps both sides of such a file, so equal keys do not settle it.
    pub untimed: u64,
    /// Folder levels below this folder (0 = no subfolder).
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
        || name == APP_TRASH_DIR_NAME
}
