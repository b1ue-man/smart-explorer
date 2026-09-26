//! The facade runtime: configuration, event hub, task threads, the backend
//! pool and the per-feature state of the handlers.
use super::config::{HostSettings, VolumeInfo};
use super::error::ApiError;
use super::events::{HubState, MAX_POLL_EVENTS};
use super::location::{self, Loc, LocKind};
use super::pool::BackendPool;
use super::slots::Slots;
use super::tasks::TaskState;
use crate::vfs::{Backend, BackendHandle, VfsMeta};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// Entries kept for `sys.errors`.
const MAX_ERROR_LOG: usize = 500;

/// A pooled backend, the backend path and its uncached metadata.
pub(crate) type Fresh = (BackendHandle, String, io::Result<VfsMeta>);

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// Event queue and task register behind one mutex, with a wake-up for
/// `poll_events`.
pub(crate) struct Hub {
    state: Mutex<HubState>,
    wake: Condvar,
}

impl Hub {
    fn new(run_tag: i64) -> Self {
        Self {
            state: Mutex::new(HubState::new(run_tag)),
            wake: Condvar::new(),
        }
    }

    pub(crate) fn with<R>(&self, apply: impl FnOnce(&mut HubState) -> R) -> R {
        let result = apply(&mut lock(&self.state));
        self.wake.notify_all();
        result
    }

    /// Waits up to `timeout` for deliverable events.
    pub(crate) fn poll(&self, timeout: Duration) -> Vec<Value> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            let now = Instant::now();
            let (events, next_due) = state.take_ready(now, MAX_POLL_EVENTS);
            if !events.is_empty() || now >= deadline {
                return events;
            }
            let until = next_due.map_or(deadline, |due| due.min(deadline));
            let wait = until
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
            state = match self.wake.wait_timeout(state, wait) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}

pub(crate) struct ErrorItem {
    pub time_ms: i64,
    pub action: String,
    pub message: String,
}

pub(crate) struct RuntimeInner {
    settings: HostSettings,
    pub(super) hub: Arc<Hub>,
    volumes: RwLock<Vec<VolumeInfo>>,
    errors: Mutex<VecDeque<ErrorItem>>,
    pub(super) pool: BackendPool,
    pub(super) transfer_slots: Arc<Slots>,
    pub(super) scans: super::scan::ScanRegistry,
    pub(super) index: super::index::IndexSlot,
    /// Serializes read-modify-write of `recent.json`.
    pub(super) recent_lock: Mutex<()>,
    /// Serializes read-modify-write of `edits.json`.
    pub(super) edits_lock: Mutex<()>,
    /// Serializes read-modify-write of `favorites.txt`.
    pub(super) favorites_lock: Mutex<()>,
}

/// The facade runtime. Cloning is cheap (shared state).
#[derive(Clone)]
pub(crate) struct Runtime {
    pub(super) inner: Arc<RuntimeInner>,
}

impl Runtime {
    /// The runtime installed by `init`.
    pub(crate) fn get() -> Result<&'static Runtime, ApiError> {
        RUNTIME
            .get()
            .ok_or_else(|| ApiError::new("not_initialized", "Kern ist noch nicht gestartet"))
    }

    /// A runtime that is not installed globally (tests, `init` before install).
    pub(crate) fn detached(settings: HostSettings) -> Self {
        let run_tag = now_ms();
        let volumes = settings.volumes.clone();
        Self {
            inner: Arc::new(RuntimeInner {
                settings,
                hub: Arc::new(Hub::new(run_tag)),
                volumes: RwLock::new(volumes),
                errors: Mutex::new(VecDeque::new()),
                pool: BackendPool::new(),
                transfer_slots: Arc::new(Slots::new(crate::transfer::MAX_ACTIVE_TRANSFERS)),
                scans: Default::default(),
                index: Default::default(),
                recent_lock: Mutex::new(()),
                edits_lock: Mutex::new(()),
                favorites_lock: Mutex::new(()),
            }),
        }
    }

    /// Installs `runtime` globally; returns the installed one (the first wins).
    pub(super) fn install(runtime: Runtime) -> &'static Runtime {
        RUNTIME.get_or_init(|| runtime)
    }

    pub(crate) fn installed() -> Option<&'static Runtime> {
        RUNTIME.get()
    }

    /// The `init` configuration (api.md §1).
    pub(crate) fn config(&self) -> &HostSettings {
        &self.inner.settings
    }

    /// The storage volumes as last reported (`init`/`sys.volumes`).
    pub(crate) fn volumes(&self) -> Vec<VolumeInfo> {
        self.inner
            .volumes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn set_volumes(&self, volumes: Vec<VolumeInfo>) {
        crate::apptrash::set_volumes(
            volumes
                .iter()
                .map(|volume| std::path::PathBuf::from(&volume.path))
                .collect(),
        );
        *self
            .inner
            .volumes
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = volumes;
    }

    /// A pooled backend and the backend path for `location`. Remote
    /// connections are opened once and reused (wrapped in `CachingBackend`).
    pub(crate) fn resolve(&self, location: &str) -> Result<(BackendHandle, String), ApiError> {
        let loc = Loc::parse(location)?;
        self.resolve_loc(&loc)
    }

    pub(crate) fn resolve_loc(&self, loc: &Loc) -> Result<(BackendHandle, String), ApiError> {
        if loc.kind == LocKind::Trash {
            return Err(ApiError::unsupported("Der Papierkorb ist kein Dateiort"));
        }
        self.inner.pool.resolve(loc)
    }

    /// Runs a read-only backend operation; a lost pooled connection is
    /// reopened and the operation retried once. Writes never retry.
    pub(crate) fn with_read<T>(
        &self,
        loc: &Loc,
        op: impl Fn(&dyn Backend, &str) -> io::Result<T>,
    ) -> Result<T, ApiError> {
        let (backend, path) = self.resolve_loc(loc)?;
        match op(&*backend, &path) {
            Err(error) if !loc.is_local() && super::error::is_connection_loss(&error) => {
                self.inner.pool.evict(loc);
                let (backend, path) = self.resolve_loc(loc)?;
                op(&*backend, &path).map_err(ApiError::from)
            }
            result => result.map_err(ApiError::from),
        }
    }

    /// The pooled backend of `loc` with its uncached metadata. A lost remote
    /// connection is evicted and reopened and the stat (a read) retried once.
    pub(crate) fn stat_fresh(&self, loc: &Loc) -> Result<Fresh, ApiError> {
        let (backend, path) = self.resolve_loc(loc)?;
        match crate::vfs::sync_backend(backend.clone()).stat(&path) {
            Err(error) if !loc.is_local() && super::error::is_connection_loss(&error) => {
                self.inner.pool.evict(loc);
                let (backend, path) = self.resolve_loc(loc)?;
                let meta = crate::vfs::sync_backend(backend.clone()).stat(&path);
                Ok((backend, path, meta))
            }
            meta => Ok((backend, path, meta)),
        }
    }

    /// A pooled backend for a write or a task. A remote connection is first
    /// checked with one uncached stat and reopened when it was lost, so a
    /// retry after a dropped session does not fail the same way again. The
    /// write itself is never retried.
    pub(crate) fn resolve_live(&self, loc: &Loc) -> Result<(BackendHandle, String), ApiError> {
        if matches!(loc.kind, LocKind::Local | LocKind::Zip) {
            return self.resolve_loc(loc);
        }
        let (backend, path, _) = self.stat_fresh(loc)?;
        Ok((backend, path))
    }

    /// Closes pooled connections whose key (`sftp://user@host:port/root`,
    /// `gdrive://`, `share://direct/<id>`, …) matches; for the domain
    /// handlers that remove a connection or device.
    #[allow(dead_code)]
    pub(crate) fn drop_backends(&self, matches: &dyn Fn(&str) -> bool) {
        self.inner.pool.retain(|key| !matches(key));
    }

    /// Queues an event for `pollEvents`.
    pub(crate) fn emit(&self, event: Value) {
        self.inner.hub.with(|state| state.push_event(event));
    }

    /// Adds an entry to the error log and emits an `error` event.
    pub(crate) fn log_error(&self, action: &str, message: &str) {
        self.record_error(action, message);
        self.emit(json!({ "type": "error", "action": action, "message": message }));
    }

    /// Adds an entry to the error log without an event.
    pub(crate) fn record_error(&self, action: &str, message: &str) {
        let mut errors = lock(&self.inner.errors);
        if errors.len() >= MAX_ERROR_LOG {
            errors.pop_front();
        }
        errors.push_back(ErrorItem {
            time_ms: now_ms(),
            action: action.to_string(),
            message: message.to_string(),
        });
    }

    pub(super) fn error_log(&self) -> Vec<Value> {
        lock(&self.inner.errors)
            .iter()
            .map(|item| {
                json!({ "timeMs": item.time_ms, "action": item.action, "message": item.message })
            })
            .collect()
    }

    pub(super) fn clear_error_log(&self) {
        lock(&self.inner.errors).clear();
    }

    /// `zip://` and `trash://` locations, rejected by every desktop format.
    pub(crate) fn is_app_internal(location: &str) -> bool {
        location::is_app_internal(location)
    }

    /// Starts `work` as a task on its own thread and returns the task id.
    pub(crate) fn spawn_task<F>(&self, kind: &str, title: String, work: F) -> String
    where
        F: FnOnce(&TaskCtx) -> Result<Value, ApiError> + Send + 'static,
    {
        self.start_task(kind, title, None, work)
    }

    /// Like `spawn_task`, but the task stays `queued` until one of the
    /// `MAX_ACTIVE_TRANSFERS` transfer slots is free.
    pub(crate) fn spawn_transfer_task<F>(&self, kind: &str, title: String, work: F) -> String
    where
        F: FnOnce(&TaskCtx) -> Result<Value, ApiError> + Send + 'static,
    {
        let slots = self.inner.transfer_slots.clone();
        self.start_task(kind, title, Some(slots), work)
    }

    fn start_task<F>(&self, kind: &str, title: String, slots: Option<Arc<Slots>>, work: F) -> String
    where
        F: FnOnce(&TaskCtx) -> Result<Value, ApiError> + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let action = title.clone();
        let id = self
            .inner
            .hub
            .with(|state| state.tasks.create(kind, title, cancel.clone(), now_ms()));
        let ctx = TaskCtx {
            id: id.clone(),
            hub: self.inner.hub.clone(),
            cancel,
            stash: Mutex::new(None),
        };
        let runtime = self.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("task-{kind}"))
            .spawn(move || {
                let _slot = match slots.as_deref() {
                    Some(slots) => match slots.acquire(&ctx.cancel) {
                        Some(slot) => Some(slot),
                        None => {
                            runtime.finish_task(&ctx, &action, Err(ApiError::canceled()));
                            return;
                        }
                    },
                    None => None,
                };
                ctx.update(|record| record.set_running());
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&ctx)))
                    .unwrap_or_else(|payload| {
                        Err(ApiError::internal(format!(
                            "Interner Fehler: {}",
                            panic_text(&*payload)
                        )))
                    });
                runtime.finish_task(&ctx, &action, outcome);
            });
        if let Err(error) = spawned {
            let message = format!("Vorgang konnte nicht gestartet werden: {error}");
            self.inner.hub.with(|state| {
                if let Some(record) = state.tasks.get_mut(&id) {
                    record.finish(TaskState::Failed, Some(message.clone()), None, now_ms());
                }
            });
            self.record_error(kind, &message);
        }
        id
    }

    fn finish_task(&self, ctx: &TaskCtx, action: &str, outcome: Result<Value, ApiError>) {
        let stash = lock(&ctx.stash).take();
        let (state, message, result) = match outcome {
            Ok(value) if ctx.cancelled() => (
                TaskState::Canceled,
                Some("Abgebrochen".to_string()),
                Some(value),
            ),
            Ok(value) => (TaskState::Done, None, Some(value)),
            Err(error) if ctx.cancelled() || error.kind == "canceled" => {
                (TaskState::Canceled, Some(error.message), stash)
            }
            Err(error) => {
                self.record_error(action, &error.message);
                (TaskState::Failed, Some(error.message), stash)
            }
        };
        ctx.update(|record| record.finish(state, message, result, now_ms()));
    }

    pub(super) fn hub(&self) -> &Hub {
        &self.inner.hub
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unbekannte Ursache".to_string()
    }
}

/// What a task's work sees: cancellation, progress, messages and errors.
pub(crate) struct TaskCtx {
    id: String,
    hub: Arc<Hub>,
    cancel: Arc<AtomicBool>,
    stash: Mutex<Option<Value>>,
}

impl TaskCtx {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub(crate) fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub(crate) fn progress(
        &self,
        done_bytes: u64,
        total_bytes: u64,
        done_items: u64,
        total_items: u64,
    ) {
        let now = Instant::now();
        self.update(|record| {
            record.progress((done_bytes, total_bytes, done_items, total_items), now)
        });
    }

    pub(crate) fn message(&self, text: &str) {
        self.update(|record| record.set_message(text));
    }

    pub(crate) fn error(&self, path: &str, message: &str) {
        self.update(|record| record.push_error(path, message));
    }

    /// The `result` of the task should it end with an error (e.g.
    /// `{conflict: true}` for `fs.uploadEdit`).
    pub(crate) fn set_failure_result(&self, value: Value) {
        *lock(&self.stash) = Some(value);
    }

    fn update(&self, apply: impl FnOnce(&mut super::tasks::TaskRecord)) {
        self.hub.with(|state| {
            if let Some(record) = state.tasks.get_mut(&self.id) {
                apply(record);
            }
        });
    }
}
