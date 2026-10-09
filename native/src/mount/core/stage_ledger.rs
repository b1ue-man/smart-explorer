//! Upload stages (`<name>.se-mount-<16 hex>`) that a mounted save created on
//! the remote and has not yet seen promoted, published or removed. The
//! ledger lives beside the recovery journal, so a stage orphaned by a lost
//! connection or an interrupted process is removed on the next save or
//! mount instead of staying next to the user's file under the stage
//! spelling. Losing the ledger loses only this cleanup, never user data:
//! the spool keeps every unsaved change.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

const LEDGER_FILE: &str = "stages.json";

#[derive(Default, Serialize, Deserialize)]
struct Recorded {
    /// Remote stage path → largest size this mount's upload gave it.
    stages: BTreeMap<String, u64>,
}

#[derive(Default)]
struct State {
    recorded: Recorded,
    /// Stages of saves still running in this process; never orphans.
    active: HashSet<String>,
}

pub(super) struct StageLedger {
    path: PathBuf,
    state: Mutex<State>,
}

/// Marks one stage as in use by a running save until dropped.
pub(super) struct ActiveStage<'a> {
    ledger: &'a StageLedger,
    stage: String,
}

impl Drop for ActiveStage<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.ledger.lock() {
            state.active.remove(&self.stage);
        }
    }
}

impl StageLedger {
    pub(super) fn open(root: &Path) -> Self {
        let path = root.join(LEDGER_FILE);
        // An unreadable ledger only forgets cleanup work (see module docs).
        let recorded = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            state: Mutex::new(State {
                recorded,
                active: HashSet::new(),
            }),
        }
    }

    /// Records `stage` before its exclusive creation and marks it active.
    /// Persisting is best effort: a save never fails over a cleanup record.
    pub(super) fn begin(&self, stage: &str, max_len: u64) -> ActiveStage<'_> {
        if let Ok(mut state) = self.lock() {
            state.active.insert(stage.to_string());
            state.recorded.stages.insert(stage.to_string(), max_len);
            let _ = self.write(&state.recorded);
        }
        ActiveStage {
            ledger: self,
            stage: stage.to_string(),
        }
    }

    /// The stage no longer exists under its spelling.
    pub(super) fn resolve(&self, stage: &str) {
        if let Ok(mut state) = self.lock() {
            if state.recorded.stages.remove(stage).is_some() {
                let _ = self.write(&state.recorded);
            }
        }
    }

    /// Recorded stages no running save owns, with their size bound.
    pub(super) fn orphans(&self) -> Vec<(String, u64)> {
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        state
            .recorded
            .stages
            .iter()
            .filter(|(stage, _)| !state.active.contains(*stage))
            .map(|(stage, max_len)| (stage.clone(), *max_len))
            .collect()
    }

    fn lock(&self) -> io::Result<MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("mount stage ledger lock poisoned"))
    }

    fn write(&self, recorded: &Recorded) -> io::Result<()> {
        let bytes = serde_json::to_vec(recorded).map_err(io::Error::other)?;
        let next = self.path.with_extension("json.new");
        let mut file = fs::File::create(&next)?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        drop(file);
        fs::rename(&next, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "se-stage-ledger-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn mount_save_task_stage_ledger_survives_restarts_and_skips_running_saves() {
        let root = directory();
        let ledger = StageLedger::open(&root);
        let running = ledger.begin("/a.pdf.se-mount-0000000000000001", 10);
        drop(ledger.begin("/b.pdf.se-mount-0000000000000002", 20));
        assert_eq!(
            ledger.orphans(),
            [("/b.pdf.se-mount-0000000000000002".to_string(), 20)],
            "a running save's stage is never an orphan"
        );
        drop(running);
        let reopened = StageLedger::open(&root);
        assert_eq!(reopened.orphans().len(), 2);
        reopened.resolve("/a.pdf.se-mount-0000000000000001");
        reopened.resolve("/missing");
        assert_eq!(
            StageLedger::open(&root).orphans(),
            [("/b.pdf.se-mount-0000000000000002".to_string(), 20)]
        );
        fs::write(root.join(LEDGER_FILE), b"not json").unwrap();
        assert!(StageLedger::open(&root).orphans().is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
