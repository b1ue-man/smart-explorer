//! Translate recorded directory existence keys through proven old relations.
//! Readonly callers only build this view; runs journal its private key delta.
use std::io;

use super::checkpoint_journal::Frame;
use super::checkpoint_run::CheckpointSink;
use super::keys::KeyPolicy;
use super::orchestration::RunState;
use super::snapshot_types::DirSet;
use super::state_spellings::StateSpellings;

pub(super) struct DirectoryHistory {
    pub dirs: Option<DirSet>,
    add: Vec<String>,
    remove: Vec<String>,
}

impl DirectoryHistory {
    pub fn load(
        names: &StateSpellings,
        recorded: Option<&DirSet>,
        keys: KeyPolicy,
    ) -> io::Result<Self> {
        let mut history = Self {
            dirs: recorded.cloned(),
            add: Vec::new(),
            remove: Vec::new(),
        };
        let Some(dirs) = &mut history.dirs else {
            return Ok(history);
        };
        if keys.fold_case {
            return Ok(history);
        }
        let folded = KeyPolicy { fold_case: true };
        for (key, logical) in &names.legacy.dirs {
            let old = folded.key(logical).into_owned();
            if old == *key || !dirs.contains(&old) || dirs.contains(key) {
                continue;
            }
            // An independently recorded current slot can own the same text
            // as an old folded key. That untyped history is ambiguous.
            if names.dirs_a.contains_key(&old) || names.dirs_b.contains_key(&old) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "historical directory key has another recorded current owner",
                ));
            }
            dirs.remove(&old);
            dirs.insert(key.clone());
            history.remove.push(old);
            history.add.push(key.clone());
        }
        Ok(history)
    }

    pub fn changed(&self) -> bool {
        !self.remove.is_empty()
    }

    pub fn checkpoint(&self, sink: &CheckpointSink<'_>) -> io::Result<()> {
        sink.planned(Frame {
            dirs_add: self.add.clone(),
            dirs_remove: self.remove.clone(),
            ..Frame::default()
        })
    }

    /// Metadata-only migration after the existing complete-cache proof. No
    /// destination enumeration or new complete index generation is needed.
    pub fn commit_noop(
        &self,
        state: &RunState<'_>,
        names: &StateSpellings,
        keys: KeyPolicy,
    ) -> io::Result<()> {
        let sink = CheckpointSink::new(
            state.endpoints,
            state.lock,
            state.key,
            keys,
            state.observer,
        )?;
        self.checkpoint(&sink)?;
        if let Some(error) = sink.finish().error {
            return Err(io::Error::other(error));
        }
        super::state_spellings::save(state.key, names)
    }
}
