use std::fmt;
use std::time::Duration;

pub(super) const MAX_TRANSACTION_ATTEMPTS: usize = 5;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CommitError {
    Conflict,
    Fatal(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TransactionError {
    Load(String),
    Mutation(String),
    Commit(String),
    ConflictsExhausted { attempts: usize },
}

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => formatter.write_str(error),
            Self::Mutation(error) => formatter.write_str(error),
            Self::Commit(error) => formatter.write_str(error),
            Self::ConflictsExhausted { attempts } => write!(
                formatter,
                "Share profiles changed concurrently during all {attempts} transaction attempts; reload and retry"
            ),
        }
    }
}

/// Reapply one idempotent, field-level mutation to the newest stored value.
///
/// `mutation` can run more than once when another process wins the optimistic
/// CAS. It must therefore avoid external side effects and produce the same
/// intended field update each time. Mutation and precondition errors are final
/// and are never retried.
pub(super) fn run<T, Load, Mutation, Commit>(
    mut load: Load,
    mut mutation: Mutation,
    mut commit: Commit,
) -> Result<T, TransactionError>
where
    Load: FnMut() -> Result<T, String>,
    Mutation: FnMut(&mut T) -> Result<(), String>,
    Commit: FnMut(&mut T) -> Result<(), CommitError>,
{
    for attempt in 0..MAX_TRANSACTION_ATTEMPTS {
        let mut current = load().map_err(TransactionError::Load)?;
        mutation(&mut current).map_err(TransactionError::Mutation)?;
        match commit(&mut current) {
            Ok(()) => return Ok(current),
            Err(CommitError::Fatal(error)) => return Err(TransactionError::Commit(error)),
            Err(CommitError::Conflict) if attempt + 1 == MAX_TRANSACTION_ATTEMPTS => {
                return Err(TransactionError::ConflictsExhausted {
                    attempts: MAX_TRANSACTION_ATTEMPTS,
                });
            }
            Err(CommitError::Conflict) => retry_delay(attempt),
        }
    }
    unreachable!("bounded profile transaction loop always returns")
}

fn retry_delay(attempt: usize) {
    let milliseconds = 1_u64 << attempt.min(4);
    std::thread::sleep(Duration::from_millis(milliseconds));
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};

    use super::{run, CommitError, TransactionError, MAX_TRANSACTION_ATTEMPTS};
    use crate::share::profiles::{ProfileRevision, ShareProfiles};

    #[derive(Default)]
    struct FakeStore {
        raw: Mutex<String>,
        forced_conflicts: AtomicUsize,
        loads: AtomicUsize,
        saves: AtomicUsize,
    }

    impl FakeStore {
        fn new(profile: &ShareProfiles) -> Self {
            Self {
                raw: Mutex::new(serde_json::to_string(profile).unwrap()),
                ..Self::default()
            }
        }

        fn load(&self) -> Result<ShareProfiles, String> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            let raw = self.raw.lock().unwrap().clone();
            let mut profile = serde_json::from_str::<ShareProfiles>(&raw).unwrap();
            profile.storage_revision = ProfileRevision::from_contents(&raw);
            Ok(profile)
        }

        fn commit(&self, profile: &mut ShareProfiles) -> Result<(), CommitError> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            if self
                .forced_conflicts
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                let mut raw = self.raw.lock().unwrap();
                let mut concurrent = serde_json::from_str::<ShareProfiles>(&raw).unwrap();
                concurrent.default_direct_exports.include_connections = true;
                *raw = serde_json::to_string(&concurrent).unwrap();
                return Err(CommitError::Conflict);
            }
            self.compare_and_swap(profile)
        }

        fn compare_and_swap(&self, profile: &mut ShareProfiles) -> Result<(), CommitError> {
            let mut raw = self.raw.lock().unwrap();
            let current = ProfileRevision::from_contents(&raw);
            if profile.storage_revision != current {
                return Err(CommitError::Conflict);
            }
            let contents = serde_json::to_string(profile).unwrap();
            profile.storage_revision = ProfileRevision::from_contents(&contents);
            *raw = contents;
            Ok(())
        }

        fn snapshot(&self) -> ShareProfiles {
            serde_json::from_str(&self.raw.lock().unwrap()).unwrap()
        }
    }

    #[test]
    fn first_save_conflict_reloads_and_rebases_the_mutation() {
        let store = FakeStore::new(&ShareProfiles::default());
        store.forced_conflicts.store(1, Ordering::SeqCst);
        let mutation_calls = AtomicUsize::new(0);

        let committed = run(
            || store.load(),
            |profile| {
                mutation_calls.fetch_add(1, Ordering::SeqCst);
                profile.auto_connect = false;
                Ok(())
            },
            |profile| store.commit(profile),
        )
        .unwrap();

        assert!(!committed.auto_connect);
        assert!(committed.default_direct_exports.include_connections);
        assert_eq!(mutation_calls.load(Ordering::SeqCst), 2);
        assert_eq!(store.saves.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn concurrent_independent_mutations_merge_after_a_cas_conflict() {
        let store = Arc::new(FakeStore::new(&ShareProfiles::default()));
        let first_loads = Arc::new(Barrier::new(2));

        let left_store = store.clone();
        let left_barrier = first_loads.clone();
        let left = std::thread::spawn(move || {
            let mut first_load = true;
            run(
                || {
                    let profile = left_store.load()?;
                    if first_load {
                        first_load = false;
                        left_barrier.wait();
                    }
                    Ok(profile)
                },
                |profile| {
                    profile.auto_connect = false;
                    Ok(())
                },
                |profile| left_store.commit(profile),
            )
            .unwrap()
        });

        let right_store = store.clone();
        let right_barrier = first_loads.clone();
        let right = std::thread::spawn(move || {
            let mut first_load = true;
            run(
                || {
                    let profile = right_store.load()?;
                    if first_load {
                        first_load = false;
                        right_barrier.wait();
                    }
                    Ok(profile)
                },
                |profile| {
                    profile.default_direct_exports.allow_exec = true;
                    Ok(())
                },
                |profile| right_store.commit(profile),
            )
            .unwrap()
        });

        left.join().unwrap();
        right.join().unwrap();
        let committed = store.snapshot();
        assert!(!committed.auto_connect);
        assert!(committed.default_direct_exports.allow_exec);
        assert!(store.saves.load(Ordering::SeqCst) >= 3);
    }

    #[test]
    fn conflict_retry_exhaustion_is_bounded() {
        let store = FakeStore::new(&ShareProfiles::default());
        store
            .forced_conflicts
            .store(MAX_TRANSACTION_ATTEMPTS, Ordering::SeqCst);

        let error = run(|| store.load(), |_| Ok(()), |profile| store.commit(profile)).unwrap_err();

        assert_eq!(
            error,
            TransactionError::ConflictsExhausted {
                attempts: MAX_TRANSACTION_ATTEMPTS
            }
        );
        assert_eq!(store.loads.load(Ordering::SeqCst), MAX_TRANSACTION_ATTEMPTS);
        assert_eq!(store.saves.load(Ordering::SeqCst), MAX_TRANSACTION_ATTEMPTS);
    }

    #[test]
    fn mutation_and_precondition_errors_are_not_retried() {
        let store = FakeStore::new(&ShareProfiles::default());
        let mutation_calls = AtomicUsize::new(0);

        let error = run(
            || store.load(),
            |_| {
                mutation_calls.fetch_add(1, Ordering::SeqCst);
                Err("contact was removed concurrently".to_string())
            },
            |profile| store.commit(profile),
        )
        .unwrap_err();

        assert_eq!(
            error,
            TransactionError::Mutation("contact was removed concurrently".into())
        );
        assert_eq!(mutation_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.loads.load(Ordering::SeqCst), 1);
        assert_eq!(store.saves.load(Ordering::SeqCst), 0);
    }
}
