//! Folders of a transfer target, each created exactly once. Workers (and sync
//! runs) ask for the folder a file goes into; the first asker creates it (its
//! parents first) under a metadata permit, everyone else waits without any
//! permit until it is ready. New top-level folders at remote targets are
//! created exclusively and get a numbered name when theirs was taken; local
//! folders are merged and never followed through links.
use super::super::engine_names::{first_component, join_rel, parent_rel, NamePlanner};
use super::super::engine_policy::{is_transient, retry_delay};
use super::super::flow::{classify_error, Flow};
use super::super::flow_control::OpOutcome;
use super::super::walk_listers::native;
use super::view::Side;
use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

const WAIT_SLICE: Duration = Duration::from_millis(100);

/// Why a folder could not be created (its kind decides whether the whole job
/// ends, for example a full or read-only target).
#[derive(Clone, Debug)]
pub(crate) struct FolderError {
    pub kind: io::ErrorKind,
    pub message: String,
}

impl FolderError {
    fn canceled() -> Self {
        Self {
            kind: io::ErrorKind::Interrupted,
            message: super::super::cancel::CANCELED_ERROR.to_string(),
        }
    }

    fn of(path: &str, error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: format!("Zielordner „{path}“ anlegen: {error}"),
        }
    }
}

#[derive(Clone)]
enum Slot {
    Creating,
    Ready { created: bool },
    Failed(FolderError),
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
                    Some(Slot::Failed(error)) => return Err(error.clone()),
                    Some(Slot::Creating) => {
                        if cancel.load(Ordering::Acquire) {
                            return Err(FolderError::canceled());
                        }
                        slots = match self.changed.wait_timeout(slots, WAIT_SLICE) {
                            Ok((guard, _)) => guard,
                            Err(poisoned) => poisoned.into_inner().0,
                        };
                    }
                    None => {
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
        let created = self.attempt(&path, cancel, |path| match self.target {
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
            let result = self.attempt(&path, cancel, |path| match self.target {
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
                        })?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// One creation under a metadata permit, retried once after a pause
    /// (without the permit) when the failure was transient.
    fn attempt(
        &self,
        path: &str,
        cancel: &AtomicBool,
        operation: impl Fn(&str) -> io::Result<bool>,
    ) -> Result<bool, FolderError> {
        let mut retried = false;
        loop {
            let permit = self
                .flow
                .acquire_meta(cancel)
                .ok_or_else(FolderError::canceled)?;
            let result = operation(path);
            permit.finish(match &result {
                Ok(_) => OpOutcome::Done,
                Err(error) => classify_error(error),
            });
            match result {
                Ok(created) => return Ok(created),
                Err(error)
                    if !retried
                        && error.kind() != io::ErrorKind::AlreadyExists
                        && is_transient(
                            error.kind(),
                            classify_error(&error) == OpOutcome::Overload,
                        ) =>
                {
                    retried = true;
                    let delay = retry_delay(
                        crate::vfs::congestion_of(&error)
                            .and_then(|congestion| congestion.retry_after),
                        super::jitter(),
                    );
                    if !super::sleep_unless(cancel, delay) {
                        return Err(FolderError::canceled());
                    }
                }
                Err(error) => return Err(FolderError::of(path, &error)),
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Creates one local folder or accepts an existing plain one; links,
/// junctions and files in its place are refused. True when created now.
pub(crate) fn ensure_plain_dir(path: &Path) -> io::Result<bool> {
    match crate::local_access::symlink_metadata(path) {
        Ok(metadata) => validate_plain_dir(path, &metadata).map(|()| false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match std::fs::create_dir(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                validate_plain_dir(path, &crate::local_access::symlink_metadata(path)?)
                    .map(|()| false)
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn validate_plain_dir(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if crate::local_access::metadata_is_link_like(path, metadata) {
        // Not a permission problem of the whole target: only what would go
        // through this link is refused.
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Zielordner ist ein Link oder Reparse-Punkt: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("Ziel ist kein Ordner: {}", path.display()),
        ));
    }
    Ok(())
}

/// The local target folder and every ancestor: plain folders (created when
/// missing), never a link that could redirect the copy elsewhere.
pub(crate) fn prepare_local_root(root: &str) -> io::Result<()> {
    let path = native(root);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Zielordner enthält „..“",
                ))
            }
            Component::Normal(name) => {
                current.push(name);
                ensure_plain_dir(&current)?;
            }
        }
    }
    Ok(())
}
