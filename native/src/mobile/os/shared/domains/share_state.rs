//! Share client state and poller. The Share service runs in the embedded
//! worker; the poller drains its events in-process (never over TCP, which
//! would split the queue) every 300 ms while the Share page is watched or a
//! pairing runs, every 5 s in the foreground and every 60 s in the
//! background, keeps the last snapshot and sends `share` / `shareRequest`.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::args::now_secs;
use super::share_exec;
use super::share_status::{open_incoming, status_json, StatusInput, WorkerFacts};
use crate::daemon::ShareWorkerSnapshot;
use crate::mobile::{ApiError, Runtime};
use crate::share::discovery_events::{
    apply_share_discovery_event, drain_discovery_command_results,
};
use crate::share::discovery_state::DiscoveryUiState;
use crate::share::{
    DiscoveryEvent, ExecProviderStatus, ProfileRevision, ShareEvent, ShareIdentity, ShareProfiles,
};

const WATCH_INTERVAL: Duration = Duration::from_millis(300);
const FOREGROUND_INTERVAL: Duration = Duration::from_secs(5);
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(60);
const MAX_NOTICES: usize = 20;
const SHARE_SERVER_FILE: &str = "share_server.txt";
/// How long a committed profile change outranks worker snapshots that do not
/// carry it yet (the worker reloads on `reconfigure`, normally at once).
const COMMIT_GUARD: Duration = Duration::from_secs(10);

/// Counts completed worker reloads (`reconfigure`); a snapshot drained while
/// one completed may predate it.
static RELOADS: AtomicU64 = AtomicU64::new(0);

/// A profile revision this process wrote and the worker has not reported yet.
struct PendingCommit {
    revision: ProfileRevision,
    until: Instant,
}

pub(super) struct ShareState {
    pub worker: Option<WorkerFacts>,
    pub poll_error: Option<String>,
    pub profiles: Option<ShareProfiles>,
    pub identity: Option<ShareIdentity>,
    pub server: Option<String>,
    pub discovery: DiscoveryUiState,
    pub offer_aliases: BTreeMap<String, String>,
    pub last_exchange: Option<String>,
    pub notices: VecDeque<String>,
    pending_commit: Option<PendingCommit>,
    seen_requests: BTreeSet<String>,
    last_status: String,
    /// Exec host activity at the last `share` event (commands started or
    /// ended on this phone are no part of the status itself).
    last_exec_activity: u64,
}

impl ShareState {
    fn new() -> Self {
        Self {
            worker: None,
            poll_error: None,
            profiles: None,
            identity: None,
            server: None,
            discovery: DiscoveryUiState::default(),
            offer_aliases: BTreeMap::new(),
            last_exchange: None,
            notices: VecDeque::new(),
            pending_commit: None,
            seen_requests: BTreeSet::new(),
            last_status: String::new(),
            last_exec_activity: 0,
        }
    }

    pub(super) fn notice(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() || self.notices.back() == Some(&text) {
            return;
        }
        if self.notices.len() == MAX_NOTICES {
            self.notices.pop_front();
        }
        self.notices.push_back(text);
    }

    /// Folds a snapshot in; returns the worker errors it carried.
    /// `reloaded_meanwhile`: the worker reloaded while this snapshot was drained.
    fn apply(
        &mut self,
        mut snapshot: ShareWorkerSnapshot,
        reloaded_meanwhile: bool,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(previous) = self.poll_error.take() {
            let stale = format!("Share-Worker nicht erreichbar: {previous}");
            if let Some(text) = crate::share::poll_status::after_successful_snapshot(
                &stale,
                snapshot.running,
                snapshot.connected,
            ) {
                self.notice(text);
            }
        }
        for event in std::mem::take(&mut snapshot.events) {
            match event {
                ShareEvent::Discovery(event) => {
                    if let Some(exchange_id) = exchange_of(&event) {
                        self.last_exchange = Some(exchange_id);
                    }
                    apply_share_discovery_event(&mut self.discovery, event);
                }
                ShareEvent::Status(text) => self.notice(text),
                ShareEvent::Error(text) => {
                    self.notice(text.clone());
                    errors.push(text);
                }
                ShareEvent::ServerDisconnected(reason) => {
                    self.notice(format!("Share-Server getrennt: {reason}"))
                }
                _ => {}
            }
        }
        // Offers of a stopped or handed-over worker end here too.
        self.discovery
            .retain_live_offers(&snapshot.discovery_offers);
        self.after_discovery_changes();
        self.worker = Some(WorkerFacts::from_snapshot(&snapshot));
        // A snapshot drained before the worker reloaded a commit still carries
        // the older profiles; the committed ones stay until the worker has it.
        let stale = reloaded_meanwhile
            || self.pending_commit.as_ref().is_some_and(|pending| {
                snapshot.profile_revision != pending.revision && Instant::now() < pending.until
            });
        if !stale {
            self.pending_commit = None;
            // Like the desktop drain: the snapshot's revision travels separately.
            let mut profiles = snapshot.profiles;
            profiles.storage_revision = snapshot.profile_revision;
            self.profiles = Some(profiles);
        }
        errors
    }

    pub(super) fn after_discovery_changes(&mut self) {
        drain_discovery_command_results(&mut self.discovery);
        self.discovery.prune_expired(now_secs());
        if let Some(status) = self.discovery.status.take() {
            self.notice(status);
        }
    }

    /// Before the first snapshot the stored profiles are shown.
    fn ensure_profiles(&mut self) {
        if self.profiles.is_some() {
            return;
        }
        match ShareProfiles::load_checked(default_home()) {
            Ok(profiles) => self.profiles = Some(profiles),
            Err(error) => {
                self.notice(format!("Share-Profile nicht verfügbar: {error}"));
                self.profiles = Some(ShareProfiles::default());
            }
        }
    }

    fn status_value(&mut self, exec_provider: &ExecProviderStatus) -> Value {
        if self.identity.is_none() {
            match ShareIdentity::load_or_create(device_name()) {
                Ok(identity) => self.identity = Some(identity),
                Err(error) => self.notice(format!("Share-Identität nicht verfügbar: {error}")),
            }
        }
        if self.server.is_none() {
            self.server = Some(read_server());
        }
        self.ensure_profiles();
        let empty = ShareProfiles::default();
        status_json(&StatusInput {
            worker: self.worker.as_ref(),
            profiles: self.profiles.as_ref().unwrap_or(&empty),
            identity: self.identity.as_ref(),
            server: self.server.as_deref().unwrap_or(""),
            poll_error: self.poll_error.as_deref(),
            discovery: &self.discovery,
            offer_aliases: &self.offer_aliases,
            last_exchange: self.last_exchange.as_deref(),
            notices: &self.notices,
            now_secs: now_secs(),
            exec_provider,
        })
    }
}

fn exchange_of(event: &DiscoveryEvent) -> Option<String> {
    match event {
        DiscoveryEvent::ExchangeStarted { exchange_id, .. }
        | DiscoveryEvent::ExchangeCompleted { exchange_id, .. }
        | DiscoveryEvent::ExchangeCancelled { exchange_id, .. } => Some(exchange_id.clone()),
        DiscoveryEvent::ExchangeFailed { exchange_id, .. } => exchange_id.clone(),
        _ => None,
    }
}

static STATE: OnceLock<Mutex<ShareState>> = OnceLock::new();

pub(super) fn with_state<T>(work: impl FnOnce(&mut ShareState) -> T) -> T {
    let state = STATE.get_or_init(|| Mutex::new(ShareState::new()));
    let mut guard: MutexGuard<'_, ShareState> =
        state.lock().unwrap_or_else(PoisonError::into_inner);
    work(&mut guard)
}

struct Cadence {
    watch: bool,
    /// A pairing command or key exchange is in flight.
    pairing: bool,
    foreground: bool,
    wake: bool,
}

static CADENCE: Mutex<Cadence> = Mutex::new(Cadence {
    watch: false,
    pairing: false,
    foreground: true,
    wake: false,
});
static WAKE: Condvar = Condvar::new();
static POLLER_STARTED: AtomicBool = AtomicBool::new(false);

fn cadence() -> MutexGuard<'static, Cadence> {
    CADENCE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Polls at once (after a command) instead of waiting for the next tick.
pub(super) fn wake() {
    cadence().wake = true;
    WAKE.notify_all();
}

pub(super) fn set_watch(active: bool) {
    let mut cadence = cadence();
    cadence.watch = active;
    cadence.wake = true;
    WAKE.notify_all();
}

pub(super) fn set_foreground(foreground: bool) {
    cadence().foreground = foreground;
    WAKE.notify_all();
}

fn wait_for_next_poll() {
    let mut guard = cadence();
    if !guard.wake {
        let interval = if guard.watch || guard.pairing {
            WATCH_INTERVAL
        } else if guard.foreground {
            FOREGROUND_INTERVAL
        } else {
            BACKGROUND_INTERVAL
        };
        guard = WAKE
            .wait_timeout(guard, interval)
            .map(|(guard, _)| guard)
            .unwrap_or_else(|poisoned| poisoned.into_inner().0);
    }
    guard.wake = false;
}

fn pairing_in_flight(discovery: &DiscoveryUiState) -> bool {
    discovery.pending_direct_publish
        || discovery.pending_room_publish.is_some()
        || !discovery.starting_discoveries.is_empty()
        || discovery.refreshing
        || discovery
            .exchanges
            .values()
            .any(|exchange| exchange.state.is_pending())
}

pub(super) fn start_poller(rt: &'static Runtime) {
    if POLLER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("share-poller".into())
        .spawn(move || loop {
            poll_once(rt);
            wait_for_next_poll();
        });
    if let Err(error) = spawned {
        POLLER_STARTED.store(false, Ordering::SeqCst);
        rt.log_error(
            "share",
            &format!("Share-Poller konnte nicht starten: {error}"),
        );
        return;
    }
    share_exec::watch_host_activity();
}

fn poll_once(rt: &Runtime) {
    let reloads = RELOADS.load(Ordering::SeqCst);
    // `None`: this process does not embed the worker (yet); nothing to drain.
    let Some(drained) = crate::daemon::drain_share_events_in_process() else {
        return;
    };
    // Outside the state lock: the first use may end leftover command trees.
    let exec_provider = share_exec::provider();
    let exec_activity = share_exec::host_activity();
    let (errors, emit_status, open_requests) = with_state(|state| {
        let errors = match drained {
            Ok(snapshot) => {
                let reloaded_meanwhile = RELOADS.load(Ordering::SeqCst) != reloads;
                state.apply(snapshot, reloaded_meanwhile)
            }
            Err(error) => {
                state.poll_error = Some(error);
                state.after_discovery_changes();
                Vec::new()
            }
        };
        let open = state
            .profiles
            .as_ref()
            .map(|profiles| open_incoming(profiles, now_secs()))
            .unwrap_or_default();
        let fresh = open.iter().any(|id| !state.seen_requests.contains(id));
        state.seen_requests = open.iter().cloned().collect();
        let status = state.status_value(exec_provider).to_string();
        let changed = status != state.last_status || exec_activity != state.last_exec_activity;
        state.last_status = status;
        state.last_exec_activity = exec_activity;
        cadence().pairing = pairing_in_flight(&state.discovery);
        (errors, changed, fresh.then_some(open.len()))
    });
    for error in errors {
        // Logged without a UI event: worker errors repeat while offline.
        rt.record_error("share", &error);
    }
    if emit_status {
        rt.emit(json!({ "type": "share" }));
    }
    if let Some(count) = open_requests {
        rt.emit(json!({ "type": "shareRequest", "count": count }));
    }
}

pub(super) fn status() -> Result<Value, ApiError> {
    let exec_provider = share_exec::provider();
    Ok(with_state(|state| state.status_value(exec_provider)))
}

/// `homeDir` of the init configuration: the Direct default export root and
/// the Share profile home, like the desktop home directory.
pub(super) fn default_home() -> Option<String> {
    crate::support_dirs::host()
        .map(|host| host.home_dir.to_string_lossy().replace('\\', "/"))
        .or_else(|| std::env::var_os("HOME").map(|home| home.to_string_lossy().replace('\\', "/")))
}

pub(super) fn device_name() -> String {
    crate::support_dirs::host()
        .map(|host| host.device_name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Mein Gerät".to_string())
}

pub(super) fn server_path() -> std::path::PathBuf {
    crate::support_dirs::app_data_file(SHARE_SERVER_FILE)
}

fn read_server() -> String {
    std::fs::read_to_string(server_path())
        .map(|server| server.trim().to_string())
        .unwrap_or_default()
}

/// The cached identity, loaded (or created) on first use.
pub(super) fn identity() -> Result<ShareIdentity, ApiError> {
    if let Some(identity) = with_state(|state| state.identity.clone()) {
        return Ok(identity);
    }
    let identity = ShareIdentity::load_or_create(device_name())
        .map_err(|error| ApiError::new("internal", error))?;
    with_state(|state| state.identity = Some(identity.clone()));
    Ok(identity)
}

/// Makes the worker reload profiles, identity and server, then polls soon.
/// Failures are logged: the change itself is already saved.
pub(super) fn reconfigure(rt: &Runtime) {
    match crate::daemon::refresh_share_worker_checked() {
        // The worker now holds the stored profiles, including changes written
        // outside `committed` (requests, removals): later snapshots are current,
        // and the stored profiles answer `share.status` until the next poll.
        Ok(_) => {
            RELOADS.fetch_add(1, Ordering::SeqCst);
            let stored = ShareProfiles::load_checked(default_home());
            with_state(|state| {
                state.pending_commit = None;
                if let Ok(profiles) = stored {
                    state.profiles = Some(profiles);
                }
            });
        }
        Err(error) => rt.log_error("share", &format!("Share-Konfiguration zustellen: {error}")),
    }
    wake();
}

/// Replaces the cached profiles after a committed change.
pub(super) fn committed(profiles: ShareProfiles) {
    with_state(|state| {
        state.pending_commit = Some(PendingCommit {
            revision: profiles.storage_revision.clone(),
            until: Instant::now() + COMMIT_GUARD,
        });
        state.profiles = Some(profiles);
    });
}

pub(super) fn set_cached_server(server: String) {
    with_state(|state| state.server = Some(server));
}
