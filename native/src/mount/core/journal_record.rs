//! Record types of the drive journal `journal.jsonl`, moved out of
//! `journal.rs` without any change of their serde form (DRV-M2), and the
//! path-sealing hook of contract V1 (S2-V3): every path field passes the
//! drive's `PathSealer` on its way to disk and back. The in-memory state is
//! always plain; the plain sealer keeps the bytes of earlier releases.
use super::path_sealer::PathSealer;
use super::types::{Baseline, EntryCondition, MountConflict, NamespaceIntent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;

pub(super) const JOURNAL_VERSION: u8 = 1;
pub(super) const MAX_RECORD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PersistedEntry {
    pub remote_path: String,
    pub spool_name: String,
    pub baseline: Baseline,
    pub condition: EntryCondition,
    #[serde(default)]
    pub delete_token: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum DeletePhase {
    LocalOnly,
    /// A dispatched quarantine move has no restart-safe commit/rollback inference.
    Unresolved,
    /// Legacy journal value; recovery treats it like `Unresolved`.
    Prepared,
    Moved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PersistedDelete {
    pub token: u64,
    pub original_path: String,
    pub quarantine_path: String,
    pub id: Option<String>,
    pub is_directory: bool,
    pub phase: DeletePhase,
}

#[derive(Clone, Default)]
pub(super) struct RecoveredJournal {
    pub entries: HashMap<String, PersistedEntry>,
    pub deletes: HashMap<u64, PersistedDelete>,
    pub namespace_conflicts: HashMap<String, NamespaceIntent>,
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u8,
    action: Action,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) enum Action {
    UpsertEntry(PersistedEntry),
    MoveEntry {
        old_path: String,
        entry: PersistedEntry,
    },
    ForgetEntry {
        remote_path: String,
    },
    UpsertDelete(PersistedDelete),
    UpsertNamespaceConflict(NamespaceIntent),
    ForgetNamespaceConflict {
        path: String,
    },
    ForgetDelete {
        token: u64,
    },
}

type PathMap<'a> = &'a dyn Fn(&str) -> io::Result<String>;

impl Action {
    /// The persisted form: every path field sealed.
    pub(super) fn sealed(self, sealer: &dyn PathSealer) -> io::Result<Self> {
        self.map_paths(&|path| sealer.seal(path))
    }

    /// The in-memory form of a persisted action.
    pub(super) fn unsealed(self, sealer: &dyn PathSealer) -> io::Result<Self> {
        self.map_paths(&|path| sealer.unseal(path))
    }

    fn map_paths(self, map: PathMap<'_>) -> io::Result<Self> {
        Ok(match self {
            Self::UpsertEntry(entry) => Self::UpsertEntry(entry.map_paths(map)?),
            Self::MoveEntry { old_path, entry } => Self::MoveEntry {
                old_path: map(old_path.as_str())?,
                entry: entry.map_paths(map)?,
            },
            Self::ForgetEntry { remote_path } => Self::ForgetEntry {
                remote_path: map(remote_path.as_str())?,
            },
            Self::UpsertDelete(delete) => Self::UpsertDelete(delete.map_paths(map)?),
            Self::UpsertNamespaceConflict(intent) => {
                Self::UpsertNamespaceConflict(map_intent(intent, map)?)
            }
            Self::ForgetNamespaceConflict { path } => Self::ForgetNamespaceConflict {
                path: map(path.as_str())?,
            },
            Self::ForgetDelete { token } => Self::ForgetDelete { token },
        })
    }
}

impl PersistedEntry {
    fn map_paths(mut self, map: PathMap<'_>) -> io::Result<Self> {
        self.remote_path = map(self.remote_path.as_str())?;
        if let EntryCondition::Conflict(conflict) = self.condition {
            self.condition = EntryCondition::Conflict(map_conflict(conflict, map)?);
        }
        Ok(self)
    }
}

impl PersistedDelete {
    fn map_paths(mut self, map: PathMap<'_>) -> io::Result<Self> {
        self.original_path = map(self.original_path.as_str())?;
        self.quarantine_path = map(self.quarantine_path.as_str())?;
        Ok(self)
    }
}

fn map_conflict(mut conflict: MountConflict, map: PathMap<'_>) -> io::Result<MountConflict> {
    conflict.path = map(conflict.path.as_str())?;
    Ok(conflict)
}

fn map_intent(mut intent: NamespaceIntent, map: PathMap<'_>) -> io::Result<NamespaceIntent> {
    intent.conflict = map_conflict(intent.conflict, map)?;
    intent.source_path = intent
        .source_path
        .map(|path| map(path.as_str()))
        .transpose()?;
    Ok(intent)
}

/// One newline-terminated journal line of an already sealed action.
pub(super) fn encode_record(action: Action) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(&Record {
        version: JOURNAL_VERSION,
        action,
    })
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mount journal record exceeds its size limit",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// The still sealed action of one journal line without its newline.
pub(super) fn decode_record(line: &[u8]) -> io::Result<Action> {
    let record: Record = serde_json::from_slice(line)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if record.version != JOURNAL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported mount journal version",
        ));
    }
    Ok(record.action)
}

pub(super) fn apply(state: &mut RecoveredJournal, action: Action) {
    match action {
        Action::UpsertEntry(entry) => {
            state.entries.insert(entry.remote_path.clone(), entry);
        }
        Action::MoveEntry { old_path, entry } => {
            state.entries.remove(&old_path);
            state.entries.insert(entry.remote_path.clone(), entry);
        }
        Action::ForgetEntry { remote_path } => {
            state.entries.remove(&remote_path);
        }
        Action::UpsertDelete(delete) => {
            state.deletes.insert(delete.token, delete);
        }
        Action::UpsertNamespaceConflict(intent) => {
            state
                .namespace_conflicts
                .insert(intent.conflict.path.clone(), intent);
        }
        Action::ForgetNamespaceConflict { path } => {
            state.namespace_conflicts.remove(&path);
        }
        Action::ForgetDelete { token } => {
            state.deletes.remove(&token);
        }
    }
}
