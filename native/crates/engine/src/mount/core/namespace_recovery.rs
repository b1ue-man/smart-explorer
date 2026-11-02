use super::commit::PathObservation;
use super::engine::{lock, MountEngine};
use super::types::{Baseline, NamespaceIntent, NamespaceOperation};
use std::io;

impl MountEngine {
    pub(super) fn retry_namespace_intents(&self) -> io::Result<()> {
        let intents = lock(&self.namespace_conflicts)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for intent in intents {
            match intent.operation {
                NamespaceOperation::CreateDirectory => self.retry_directory_create(&intent)?,
                NamespaceOperation::RenameNoReplace => self.retry_no_replace_rename(&intent)?,
            }
        }
        Ok(())
    }

    fn retry_directory_create(&self, intent: &NamespaceIntent) -> io::Result<()> {
        let path = &intent.conflict.path;
        match self.observe_path(path) {
            PathObservation::Missing => self.forget_namespace_conflict(path),
            PathObservation::Present {
                is_dir: true,
                is_symlink: false,
                ..
            } => self.forget_namespace_conflict(path),
            observation => Err(unresolved(format!(
                "directory create at {path} is still ambiguous: {}",
                observation.summary()
            ))),
        }
    }

    fn retry_no_replace_rename(&self, intent: &NamespaceIntent) -> io::Result<()> {
        let source = intent.source_path.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "rename recovery intent has no source path",
            )
        })?;
        let baseline = intent.source_baseline.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "rename recovery intent has no source baseline",
            )
        })?;
        let is_directory = intent.source_is_directory.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "rename recovery intent has no source type",
            )
        })?;
        let destination = &intent.conflict.path;
        let source_state = self.observe_path(source);
        let destination_state = self.observe_path(destination);
        let not_committed = source_state.matches(baseline, Some(is_directory))
            && destination_state.matches(&Baseline::Missing, None);
        if not_committed {
            return self.forget_namespace_conflict(destination);
        }
        let committed = matches!(source_state, PathObservation::Missing)
            && destination_state.matches(baseline, Some(is_directory));
        if !committed {
            return Err(unresolved(format!(
                "rename {source} -> {destination} is still ambiguous: source={}, destination={}",
                source_state.summary(),
                destination_state.summary()
            )));
        }
        self.reconcile_renamed_entries(source, destination)?;
        self.forget_namespace_conflict(destination)
    }

    fn reconcile_renamed_entries(&self, source: &str, destination: &str) -> io::Result<()> {
        let cached = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        let mut affected = Vec::new();
        for entry in cached {
            let state = lock(&entry.state)?;
            if let Some(suffix) = self.descendant_suffix(&state.remote_path, source) {
                affected.push((
                    entry.clone(),
                    state.remote_path.clone(),
                    format!("{destination}{suffix}"),
                ));
            }
        }
        for (entry, old_path, new_path) in &affected {
            let state = lock(&entry.state)?;
            if state.condition != super::types::EntryCondition::Clean
                || state.delete_token.is_some()
            {
                let mut moved = state.persisted();
                moved.remote_path = new_path.clone();
                self.spool.move_entry(old_path, &moved)?;
            }
        }
        for (entry, _, new_path) in &affected {
            lock(&entry.state)?.remote_path = new_path.clone();
        }
        let mut entries = lock(&self.entries)?;
        for (entry, old_path, new_path) in affected {
            entries.remove(&self.cache_key(&old_path));
            entries.insert(self.cache_key(&new_path), entry);
        }
        Ok(())
    }
}

fn unresolved(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, message)
}
