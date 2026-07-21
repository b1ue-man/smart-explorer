use super::engine::{baseline_from_meta, lock, EntryState, MountEngine};
use super::types::{Baseline, EntryCondition, MountConflict, NamespaceIntent};
use std::io;

#[derive(Clone, Debug)]
pub(super) enum PathObservation {
    Missing,
    Present {
        baseline: Baseline,
        is_dir: bool,
        is_symlink: bool,
    },
    Unknown(String),
}

impl PathObservation {
    pub(super) fn matches(&self, expected: &Baseline, expected_is_dir: Option<bool>) -> bool {
        match (self, expected) {
            (Self::Missing, Baseline::Missing) => true,
            (
                Self::Present {
                    baseline,
                    is_dir,
                    is_symlink,
                },
                expected @ Baseline::Present { .. },
            ) => {
                !is_symlink
                    && expected_is_dir.map_or(true, |expected_dir| expected_dir == *is_dir)
                    && baseline == expected
            }
            _ => false,
        }
    }

    pub(super) fn current(&self) -> Option<Baseline> {
        match self {
            Self::Present { baseline, .. } => Some(baseline.clone()),
            Self::Missing | Self::Unknown(_) => None,
        }
    }

    pub(super) fn is_plain_file(&self) -> bool {
        matches!(
            self,
            Self::Present {
                is_dir: false,
                is_symlink: false,
                ..
            }
        )
    }

    pub(super) fn summary(&self) -> String {
        match self {
            Self::Missing => "missing".into(),
            Self::Present {
                is_dir, is_symlink, ..
            } => format!("present(dir={is_dir}, link={is_symlink})"),
            Self::Unknown(detail) => format!("unknown({detail})"),
        }
    }
}

impl MountEngine {
    pub(super) fn observe_path(&self, path: &str) -> PathObservation {
        match self.backend.stat(path) {
            Ok(metadata) => PathObservation::Present {
                baseline: baseline_from_meta(&metadata),
                is_dir: metadata.is_dir,
                is_symlink: metadata.is_symlink,
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => PathObservation::Missing,
            Err(error) => PathObservation::Unknown(error.to_string()),
        }
    }

    pub(super) fn persist_namespace_intent(&self, intent: &NamespaceIntent) -> io::Result<()> {
        let key = self.cache_key(&intent.conflict.path);
        if lock(&self.namespace_conflicts)?
            .get(&key)
            .is_some_and(|existing| existing.conflict.path != intent.conflict.path)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "case-aliasing namespace conflicts are not uniquely representable",
            ));
        }
        self.spool.persist_namespace_conflict(intent)?;
        lock(&self.namespace_conflicts)?.insert(key, intent.clone());
        Ok(())
    }

    pub(super) fn retain_namespace_conflict(&self, conflict: MountConflict) {
        // A pre-dispatch namespace marker is already durable. Enriching it is
        // best effort after an ambiguous response, just like an entry conflict.
        let key = self.cache_key(&conflict.path);
        if lock(&self.namespace_conflicts)
            .ok()
            .and_then(|conflicts| {
                conflicts
                    .get(&key)
                    .map(|existing| existing.conflict.path.clone())
            })
            .is_some_and(|path| path != conflict.path)
        {
            return;
        }
        if let Ok(mut conflicts) = lock(&self.namespace_conflicts) {
            if let Some(intent) = conflicts.get_mut(&key) {
                intent.conflict = conflict;
                let _ = self.spool.persist_namespace_conflict(intent);
            }
        }
    }

    pub(super) fn forget_namespace_conflict(&self, path: &str) -> io::Result<()> {
        let key = self.cache_key(path);
        let persisted_path = lock(&self.namespace_conflicts)?
            .get(&key)
            .map(|intent| intent.conflict.path.clone())
            .unwrap_or_else(|| path.to_string());
        self.spool.forget_namespace_conflict(&persisted_path)?;
        lock(&self.namespace_conflicts)?.remove(&key);
        Ok(())
    }

    pub(super) fn namespace_conflict_exists(&self, path: &str) -> io::Result<bool> {
        Ok(lock(&self.namespace_conflicts)?.contains_key(&self.cache_key(path)))
    }

    pub(super) fn post_commit_conflict(
        &self,
        state: &mut EntryState,
        current: Option<Baseline>,
        detail: &str,
    ) -> MountConflict {
        let conflict = MountConflict {
            path: state.remote_path.clone(),
            baseline: state.baseline.clone(),
            current,
            detail: detail.to_string(),
        };
        // A pre-promotion Dirty record is already durable. Updating it to the
        // richer Conflict is best effort; an acknowledged remote commit must
        // not be reported to Windows as though it never happened.
        let _ = self
            .spool
            .persist_entry(&state.with_condition(EntryCondition::Conflict(conflict.clone())));
        state.condition = EntryCondition::Conflict(conflict.clone());
        conflict
    }
}
