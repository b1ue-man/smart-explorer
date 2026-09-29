//! Folders of a transfer target, each created exactly once. Workers (and sync
//! runs) ask for the folder a file goes into; the first asker creates it (its
//! parents first) under a metadata permit, everyone else waits without any
//! permit until it is ready. New top-level folders at remote targets are
//! created exclusively and get a numbered name when theirs was taken; local
//! folders are merged and never followed through links. A peer that is too
//! busy is waited for while folders of this register still get created
//! (K13), and such a refusal is never kept as the folder's lasting failure;
//! an exclusive creation whose answer was lost is looked at before it is
//! tried again, so no empty folder stays behind while the files land in
//! "Name (2)".
use super::super::engine_names::{first_component, join_rel, parent_rel, NamePlanner};
use super::super::engine_policy::{
    connection_failure, is_back_pressure, is_transient, overload_keeps_waiting, retry_delay,
    surely_not_done, RETRY_AFTER_MAX,
};
use super::super::flow::{classify_error, Flow};
use super::super::flow_control::OpOutcome;
use super::super::walk_listers::native;
use super::view::Side;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[path = "folders_local.rs"]
mod local;
pub(crate) use local::{ensure_plain_dir, local_target_root};

const WAIT_SLICE: Duration = Duration::from_millis(100);

/// Why a folder could not be created (its kind decides whether the whole job
/// ends, for example a full or read-only target).
#[derive(Clone, Debug)]
pub(crate) struct FolderError {
    pub kind: io::ErrorKind,
    pub message: String,
    /// This call saw the failure itself and it speaks about the connection:
    /// it counts once for the job's breaker; later askers of the failed
    /// folder get it without.
    pub connection: bool,
    /// The peer kept refusing as too busy (back-pressure, not a problem of
    /// the folder): a later asker tries again.
    pub congestion: bool,
}

impl FolderError {
    fn canceled() -> Self {
        Self {
            kind: io::ErrorKind::Interrupted,
            message: super::super::cancel::CANCELED_ERROR.to_string(),
            connection: false,
            congestion: false,
        }
    }

    fn of(path: &str, error: &io::Error) -> Self {
        let overload = classify_error(error) == OpOutcome::Overload;
        Self {
            kind: error.kind(),
            message: format!("Zielordner „{path}“ anlegen: {error}"),
            connection: connection_failure(error.kind(), overload),
            congestion: is_back_pressure(error.kind(), overload),
        }
    }

    /// The failure as a later asker of the folder gets it.
    fn repeated(&self) -> Self {
        Self {
            connection: false,
            ..self.clone()
        }
    }
}

/// What an exclusive creation whose answer was lost left behind.
enum Settled {
    /// Nothing: it did not happen.
    Missing,
    /// An empty folder: most likely the lost creation itself.
    Empty,
    /// Something else took the name.
    Taken,
}

#[derive(Clone)]
enum Slot {
    Creating,
    Ready {
        created: bool,
    },
    Failed(FolderError),
    /// Refused as too busy: askers until `until` get the failure, later ones
    /// create again.
    Refused {
        error: FolderError,
        until: Instant,
    },
}

/// The folders below one target folder.
pub(crate) struct FolderRegister<'a> {
    target: Side<'a>,
    root: String,
    flow: Arc<Flow>,
    /// Remote targets: free names for new top-level entries.
    planner: Option<Mutex<NamePlanner>>,
    /// Planned top-level folder → the name it was planned from; created
    /// exclusively and renumbered when taken meanwhile.
    exclusive: Mutex<HashMap<String, String>>,
    /// Planned top-level name → the name it actually got.
    aliases: Mutex<HashMap<String, String>>,
    slots: Mutex<HashMap<String, Slot>>,
    changed: Condvar,
    /// The last folder this register created: progress that lets a refused
    /// creation keep waiting.
    last_created: Mutex<Option<Instant>>,
}

impl<'a> FolderRegister<'a> {
    /// Folders below `root` (the target folder, which must exist) on
    /// `target`; creations take metadata permits on `flow`.
    pub(crate) fn new(target: Side<'a>, root: &str, flow: Arc<Flow>) -> Self {
        Self {
            target,
            root: root.to_string(),
            flow,
            planner: None,
            exclusive: Mutex::new(HashMap::new()),
            aliases: Mutex::new(HashMap::new()),
            slots: Mutex::new(HashMap::new()),
            changed: Condvar::new(),
            last_created: Mutex::new(None),
        }
    }

    /// New top-level names are chosen against these existing names.
    pub(crate) fn with_planner(mut self, planner: NamePlanner) -> Self {
        self.planner = Some(Mutex::new(planner));
        self
    }

    /// A free top-level name for `wanted` (remote targets with a planner;
    /// elsewhere `wanted` itself).
    pub(crate) fn claim_name(&self, wanted: &str) -> Option<String> {
        match &self.planner {
            Some(planner) => lock(planner).claim(wanted),
            None => Some(wanted.to_string()),
        }
    }

    /// `taken` exists after all: the next free top-level name for `wanted`.
    pub(crate) fn reclaim_name(&self, wanted: &str, taken: &str) -> Option<String> {
        match &self.planner {
            Some(planner) => lock(planner).reclaim(wanted, taken),
            None => None,
        }
    }

    /// `planned` (from `claim_name(wanted)`) is a new top-level folder: it is
    /// created exclusively, never merged into an existing one.
    pub(crate) fn plan_exclusive(&self, planned: &str, wanted: &str) {
        lock(&self.exclusive).insert(planned.to_string(), wanted.to_string());
    }

    /// A top-level entry `planned` was published as `actual`.
    pub(crate) fn record_alias(&self, planned: &str, actual: &str) {
        if planned != actual {
            lock(&self.aliases).insert(planned.to_string(), actual.to_string());
        }
    }

    /// The target folder everything lands below.
    pub(crate) fn root(&self) -> &str {
        &self.root
    }

    /// Target path of `rel` (top-level renumbering applied).
    pub(crate) fn path_of(&self, rel: &str) -> String {
        join_rel(&self.root, &self.actual_rel(rel))
    }

    /// `rel` with its top-level component as actually created.
    pub(crate) fn actual_rel(&self, rel: &str) -> String {
        let first = first_component(rel);
        match lock(&self.aliases).get(first) {
            Some(actual) => super::super::engine_names::with_first(rel, actual),
            None => rel.to_string(),
        }
    }

    /// Makes sure folder `rel` exists; true when this register created it or
    /// one of its parents (nothing foreign can be inside then).
    pub(crate) fn ensure(&self, rel: &str, cancel: &AtomicBool) -> Result<bool, FolderError> {
        if rel.is_empty() {
            return Ok(false);
        }
        {
            let mut slots = lock(&self.slots);
            loop {
                match slots.get(rel) {
                    Some(Slot::Ready { created }) => return Ok(*created),
                    Some(Slot::Failed(error)) => return Err(error.repeated()),
                    Some(Slot::Refused { error, until }) if Instant::now() < *until => {
                        return Err(error.repeated())
                    }
                    Some(Slot::Creating) => {
                        if cancel.load(Ordering::Acquire) {
                            return Err(FolderError::canceled());
                        }
                        slots = match self.changed.wait_timeout(slots, WAIT_SLICE) {
                            Ok((guard, _)) => guard,
                            Err(poisoned) => poisoned.into_inner().0,
                        };
                    }
                    Some(Slot::Refused { .. }) | None => {
                        slots.insert(rel.to_string(), Slot::Creating);
                        break;
                    }
                }
            }
        }
        let result = self.create(rel, cancel);
        {
            let mut slots = lock(&self.slots);
            match &result {
                Ok(created) => {
                    slots.insert(rel.to_string(), Slot::Ready { created: *created });
                }
                Err(error) if error.kind == io::ErrorKind::Interrupted => {
                    slots.remove(rel);
                }
                // The askers waiting now get the refusal; after the longest
                // wait a peer may ask for, the next asker creates again.
                Err(error) if error.congestion => {
                    let refused = Slot::Refused {
                        error: error.clone(),
                        until: Instant::now() + RETRY_AFTER_MAX,
                    };
                    slots.insert(rel.to_string(), refused);
                }
                Err(error) => {
                    slots.insert(rel.to_string(), Slot::Failed(error.clone()));
                }
            }
        }
        self.changed.notify_all();
        result
    }

    fn create(&self, rel: &str, cancel: &AtomicBool) -> Result<bool, FolderError> {
        let parent_created = match parent_rel(rel) {
            Some(parent) => self.ensure(parent, cancel)?,
            None => false,
        };
        let wanted = lock(&self.exclusive).get(rel).cloned();
        if let Some(wanted) = wanted {
            return self.create_exclusive(rel, &wanted, cancel);
        }
        let path = self.path_of(rel);
        let created = self.attempt(&path, cancel, false, |path| match self.target {
            Side::Remote(backend) => backend.create_dir(path).map(|()| parent_created),
            Side::Local => ensure_plain_dir(&native(path)),
        })?;
        Ok(created || parent_created)
    }

    fn create_exclusive(
        &self,
        planned: &str,
        wanted: &str,
        cancel: &AtomicBool,
    ) -> Result<bool, FolderError> {
        let mut name = planned.to_string();
        loop {
            let path = join_rel(&self.root, &name);
            let result = self.attempt(&path, cancel, true, |path| match self.target {
                Side::Remote(backend) => backend.create_dir_new(path).map(|()| true),
                Side::Local => ensure_plain_dir(&native(path)),
            });
            match result {
                Ok(created) => {
                    lock(&self.aliases).insert(planned.to_string(), name);
                    return Ok(created);
                }
                Err(error) if error.kind == io::ErrorKind::AlreadyExists => {
                    name = self
                        .reclaim_name(wanted, &name)
                        .ok_or_else(|| FolderError {
                            kind: io::ErrorKind::AlreadyExists,
                            message: format!("Kein freier Name für „{wanted}“ gefunden"),
                            connection: false,
                            congestion: false,
                        })?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// One creation under a metadata permit. A peer that is too busy is
    /// waited for (the permit goes back as overload, the pause follows the
    /// peer's own delay) until the patience ran out without any folder of
    /// this register created; another transient failure is retried once
    /// after a pause. An `exclusive` creation is never simply repeated after
    /// a failure that may have created the folder with only the answer lost.
    fn attempt(
        &self,
        path: &str,
        cancel: &AtomicBool,
        exclusive: bool,
        operation: impl Fn(&str) -> io::Result<bool>,
    ) -> Result<bool, FolderError> {
        let mut retried = false;
        let mut refused_since: Option<Instant> = None;
        loop {
            let error = match self.metadata_op(cancel, || operation(path))? {
                Ok(created) => {
                    *lock(&self.last_created) = Some(Instant::now());
                    return Ok(created);
                }
                Err(error) => error,
            };
            let kind = error.kind();
            let overload = classify_error(&error) == OpOutcome::Overload;
            let back_pressure = is_back_pressure(kind, overload);
            let wait = if back_pressure {
                let since = *refused_since.get_or_insert_with(Instant::now);
                let progress = *lock(&self.last_created);
                overload_keeps_waiting(progress.map_or(since, |last| since.max(last)).elapsed())
            } else {
                !retried && kind != io::ErrorKind::AlreadyExists && is_transient(kind, overload)
            };
            if !wait {
                return Err(FolderError::of(path, &error));
            }
            retried |= !back_pressure;
            let delay = retry_delay(
                crate::vfs::congestion_of(&error).and_then(|congestion| congestion.retry_after),
                super::jitter(),
            );
            if !super::sleep_unless(cancel, delay) {
                return Err(FolderError::canceled());
            }
            if exclusive && !surely_not_done(kind, overload) {
                match self.settle(path, cancel)? {
                    Settled::Missing => {}
                    // Adopted, but not trusted to hold nothing foreign: its
                    // files still go through stages and create-only names.
                    Settled::Empty => return Ok(false),
                    Settled::Taken => {
                        return Err(FolderError {
                            kind: io::ErrorKind::AlreadyExists,
                            ..FolderError::of(path, &error)
                        })
                    }
                }
            }
        }
    }

    /// Looks at `path` after an exclusive creation whose answer was lost.
    fn settle(&self, path: &str, cancel: &AtomicBool) -> Result<Settled, FolderError> {
        let Side::Remote(backend) = self.target else {
            // Local creation merges plain folders; repeating it is safe.
            return Ok(Settled::Missing);
        };
        let meta = match self.metadata_op(cancel, || backend.stat(path))? {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Settled::Missing),
            Err(error) => return Err(FolderError::of(path, &error)),
        };
        if !meta.is_dir || meta.is_symlink {
            return Ok(Settled::Taken);
        }
        match self.metadata_op(cancel, || backend.list_dir(path))? {
            Ok(entries) if entries.is_empty() => Ok(Settled::Empty),
            Ok(_) => Ok(Settled::Taken),
            Err(error) => Err(FolderError::of(path, &error)),
        }
    }

    /// One metadata operation under a permit of the target's flow.
    fn metadata_op<T>(
        &self,
        cancel: &AtomicBool,
        operation: impl FnOnce() -> io::Result<T>,
    ) -> Result<io::Result<T>, FolderError> {
        let permit = self
            .flow
            .acquire_meta(cancel)
            .ok_or_else(FolderError::canceled)?;
        let result = operation();
        permit.finish(match &result {
            Ok(_) => OpOutcome::Done,
            Err(error) => classify_error(error),
        });
        Ok(result)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "folders_tests.rs"]
mod tests;
