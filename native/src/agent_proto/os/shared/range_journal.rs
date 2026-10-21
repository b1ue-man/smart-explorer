//! A host's list of range stages that are not finished (E13-S2, "beim Start"):
//! a stage that a crash or a killed process left behind is removed when the
//! next process of the same folder starts. Every process writes its own list
//! file and keeps an exclusive lock on it while it lives, so a starting
//! process sweeps exactly the lists whose owner is gone and never the stages
//! of a process that still runs (several agent channels, the desktop app and
//! the service). Best effort: a list that cannot be written never stops a
//! transfer, and a path is removed only while it still is a regular file with
//! the exact name of a stage this code makes.
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The lists of this feature are named `<PREFIX><pid>-<random>.list`.
const PREFIX: &str = "se-range-stages-";
const SUFFIX: &str = ".list";
/// A list is rewritten once it grows beyond this.
const COMPACT_ABOVE: u64 = 1024 * 1024;

struct State {
    file: File,
    open: BTreeSet<String>,
}

pub(crate) struct Journal {
    state: Mutex<State>,
}

impl Journal {
    /// Sweeps the lists of finished processes in `dir`, then opens (and locks)
    /// the list of this process. `None` when `dir` cannot hold it.
    pub(crate) fn open(dir: PathBuf) -> Option<Arc<Self>> {
        super::local_platform::prepare_private_dir(&dir).ok()?;
        sweep_dead_lists(&dir);
        let mut random = [0u8; 8];
        getrandom::getrandom(&mut random).ok()?;
        let name = format!(
            "{PREFIX}{}-{:016x}{SUFFIX}",
            std::process::id(),
            u64::from_le_bytes(random)
        );
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .open(dir.join(name))
            .ok()?;
        file.try_lock().ok()?;
        Some(Arc::new(Self {
            state: Mutex::new(State {
                file,
                open: BTreeSet::new(),
            }),
        }))
    }

    /// A stage was created and is unfinished.
    pub(crate) fn add(&self, stage: &Path) {
        self.record('+', stage);
    }

    /// A stage was finished, discarded or removed.
    pub(crate) fn done(&self, stage: &Path) {
        self.record('-', stage);
    }

    fn record(&self, mark: char, stage: &Path) {
        let Some(text) = stage.to_str().filter(|text| !text.contains('\n')) else {
            return;
        };
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if mark == '+' {
            state.open.insert(text.to_string());
        } else {
            state.open.remove(text);
        }
        let _ = writeln!(state.file, "{mark}{text}");
        let large = state
            .file
            .metadata()
            .is_ok_and(|meta| meta.len() > COMPACT_ABOVE);
        if large {
            let mut text = String::new();
            for stage in &state.open {
                text.push('+');
                text.push_str(stage);
                text.push('\n');
            }
            if state.file.set_len(0).is_ok() {
                let _ = state.file.write_all(text.as_bytes());
            }
        }
    }
}

/// The names a list's text still holds as unfinished.
fn unfinished(text: &str) -> BTreeSet<&str> {
    let mut open = BTreeSet::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix('+') {
            open.insert(path);
        } else if let Some(path) = line.strip_prefix('-') {
            open.remove(path);
        }
    }
    open
}

/// Removes the stages of every list in `dir` whose owner no longer holds
/// its lock, and the list itself.
fn sweep_dead_lists(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(PREFIX) || !name.ends_with(SUFFIX) {
            continue;
        }
        let path = entry.path();
        let Ok(mut file) = OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        // A lock that is taken belongs to a process that still runs.
        if file.try_lock().is_err() {
            continue;
        }
        let mut text = String::new();
        if std::io::Read::read_to_string(&mut file, &mut text).is_ok() {
            for stage in unfinished(&text) {
                remove_orphan(Path::new(stage));
            }
        }
        drop(file);
        let _ = std::fs::remove_file(&path);
    }
}

/// Removes `path` if it still is a regular file named like a stage.
fn remove_orphan(path: &Path) {
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(crate::vfs::is_unique_stage);
    if !named {
        return;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_file() && !super::local_platform::metadata_is_link_like(path, &metadata) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage_files(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let open = dir.join("a.bin.se-upload-0123456789abcdef");
        let done = dir.join("b.bin.se-upload-fedcba9876543210");
        let user = dir.join("keep.txt");
        for path in [&open, &done, &user] {
            std::fs::write(path, b"x").expect("fixture");
        }
        (open, done, user)
    }

    #[test]
    fn speed_task_range_journal_removes_unfinished_stages_of_a_dead_process() {
        let dir = tempfile::tempdir().expect("temp dir");
        let lists = dir.path().join("lists");
        let (open, done, user) = stage_files(dir.path());
        let dead = Journal::open(lists.clone()).expect("journal");
        dead.add(&open);
        dead.add(&done);
        dead.add(&user);
        dead.done(&done);
        drop(dead);

        let restarted = Journal::open(lists).expect("journal");
        assert!(!open.exists(), "an unfinished stage is removed at start");
        assert!(done.exists(), "a finished stage is not the journal's");
        assert!(
            user.exists(),
            "a file that is no stage name is never removed"
        );
        drop(restarted);
    }

    #[test]
    fn speed_task_range_journal_keeps_the_stages_of_a_live_process() {
        let dir = tempfile::tempdir().expect("temp dir");
        let lists = dir.path().join("lists");
        let (open, _, _) = stage_files(dir.path());
        let live = Journal::open(lists.clone()).expect("journal");
        live.add(&open);
        let second = Journal::open(lists).expect("journal");
        assert!(open.exists(), "its owner still holds the lock");
        drop((live, second));
    }
}
