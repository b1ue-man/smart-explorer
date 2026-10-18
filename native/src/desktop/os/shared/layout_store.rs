//! The only writer of `<AD>/desktop/layout/<Monitorschlüssel>.json` (C5.4,
//! C5.5): the shell holds one store. At its start the store loads every valid
//! arrangement; after a change it writes each changed monitor atomically
//! (temporary file and rename, `write_private_atomic`) and deletes the file
//! of a monitor the arrangement no longer has (its items went to the primary
//! monitor). Surfaces never write: they send `layout {op}` to the shell.
//! A process-wide lock orders the writes of the stores of one process; the
//! shell's single-instance lock keeps every other process out. Nothing is
//! written while se Desktop is absent, so a removal is never undone.
//!
//! Arrangement files hold item IDs only, never a path or a name from inside
//! a vault (S5-V5).
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use super::store::DesktopDir;
use crate::desktop::core::items::{self, DesktopFile};
use crate::desktop::core::layout::{DesktopLayout, ItemId, MonitorLayout};

/// Bound of one arrangement file.
const MAX_LAYOUT: u64 = 256 * 1024;
/// Orders the writes of every store of this process.
static WRITES: Mutex<()> = Mutex::new(());

pub(crate) struct LayoutStore {
    dir: DesktopDir,
    /// What each file holds after the last load or write.
    written: BTreeMap<String, MonitorLayout>,
    /// The shell's model revision of the last `save_at`.
    saved_rev: Option<u64>,
}

impl LayoutStore {
    /// The arrangements of `layout/` and the store that keeps them. A file
    /// that cannot be read or names another monitor is logged and stays until
    /// its monitor is written again.
    pub(crate) fn open(dir: DesktopDir) -> (LayoutStore, DesktopLayout) {
        let mut layout = DesktopLayout::default();
        let mut written = BTreeMap::new();
        if let Ok(entries) = std::fs::read_dir(dir.layout_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(key) = monitor_key_of(&path) else {
                    continue;
                };
                match read_layout(&path) {
                    Ok(monitor) if monitor.monitor == key => {
                        written.insert(key.clone(), monitor.clone());
                        layout.monitors.insert(key, monitor);
                    }
                    Ok(monitor) => dir.log(
                        "layout_ignored",
                        &format!("{} gehört zu {}", path.display(), monitor.monitor),
                    ),
                    Err(error) => {
                        dir.log("layout_unreadable", &format!("{}: {error}", path.display()));
                    }
                }
            }
        }
        let store = LayoutStore {
            dir,
            written,
            saved_rev: None,
        };
        (store, layout)
    }

    /// [`save`](Self::save) once per model revision of the shell (`rev`
    /// grows with every change of the arrangement).
    pub(crate) fn save_at(&mut self, layout: &DesktopLayout, rev: u64) -> Result<usize, String> {
        if self.saved_rev == Some(rev) {
            return Ok(0);
        }
        self.saved_rev = Some(rev);
        self.save(layout)
    }

    /// Writes every monitor that changed since the last call and deletes the
    /// files of monitors that left; `Ok(n)`: files written or deleted.
    pub(crate) fn save(&mut self, layout: &DesktopLayout) -> Result<usize, String> {
        if !self.dir.exists() {
            return Ok(0);
        }
        let _order = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let folder = self.dir.layout_dir();
        let mut changed = 0;
        for (key, monitor) in &layout.monitors {
            if self.written.get(key) == Some(monitor) {
                continue;
            }
            let text = monitor.to_json().map_err(|error| error.to_string())?;
            let path = folder.join(monitor.file_name());
            crate::support_dirs::write_private_atomic(&path, text.as_bytes())
                .map_err(|error| format!("{}: {error}", path.display()))?;
            self.written.insert(key.clone(), monitor.clone());
            changed += 1;
        }
        let gone: Vec<String> = self
            .written
            .keys()
            .filter(|key| !layout.monitors.contains_key(*key))
            .cloned()
            .collect();
        for key in gone {
            let path = folder.join(format!("{key}.json"));
            match std::fs::remove_file(&path) {
                Ok(()) => changed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("{}: {error}", path.display())),
            }
            self.written.remove(&key);
        }
        Ok(changed)
    }
}

/// Desktop files that left their folder leave the arrangement; devices,
/// rooms and vaults keep their cell (C5.4).
pub(crate) fn forget_missing_files(layout: &mut DesktopLayout, files: &[DesktopFile]) {
    let present: Vec<ItemId> = items::visible_files(files)
        .iter()
        .map(DesktopFile::item_id)
        .collect();
    layout.forget_missing_files(&present);
}

/// `<key>.json` of the arrangement folder; temporary files never match.
fn monitor_key_of(path: &Path) -> Option<String> {
    if path.extension()? != "json" {
        return None;
    }
    let key = path.file_stem()?.to_str()?;
    (!key.starts_with('.')).then(|| key.to_string())
}

fn read_layout(path: &Path) -> Result<MonitorLayout, String> {
    let text = crate::support_dirs::read_private_text(path, MAX_LAYOUT)
        .map_err(|error| error.to_string())?;
    MonitorLayout::from_json(&text).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "layout_store_tests.rs"]
mod tests;
