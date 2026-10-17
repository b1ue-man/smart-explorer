//! The session register (VLT-M4, C3): the unlocked vaults of the process
//! that holds the keys — the background worker on the desktop, the app
//! process on Android. One unlock serves every surface of the device: GUI,
//! CLI and MCP reach the worker's sessions over IPC (`Vault::*`), the
//! Android facade calls this register directly.
//!
//! - Register: one `VaultSession` per unlocked site, published to the
//!   process table of `open.rs`, so `vault://` opens with it.
//! - Epoch: every unlock gets a new number; replies, events and consumers
//!   carry it, so a holder tells a later unlock apart.
//! - Consumers: what holds a session besides single operations (C3.3):
//!   `Vault::Open` connections, drives, comparisons, fallback copies.
//! - Lock participants: called with a deadline before the key is wiped.
//!
//! Locking runs the participants, takes the session out of the register and
//! the process table, then wipes the key (`VaultSession::lock`): every open
//! backend, reader and writer fails with `VaultLocked` from then on. The
//! automatic triggers are in `session_timer.rs`.
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::vault::core::lock_policy::{LockPlatform, LockReason, SignalControl};
use crate::vault::core::registry_types::{LockSettings, SiteRecord};
use crate::vault::core::VaultSession;

use super::open::{publish_session, withdraw_session};

#[path = "session_seal.rs"]
mod seal;
#[path = "session_timer.rs"]
mod timer;
#[path = "session_types.rs"]
mod types;

pub(crate) use types::{
    ConsumerKind, Installed, LockError, LockParticipant, LockRequest, RegisterEvent,
    RegisterOptions, SiteState, SiteStatus,
};

/// Events kept for clients that poll (`Vault::Status`, S5 H5).
pub(crate) const EVENT_LOG: usize = 64;
/// A lock or an unlock waits this long for a lock of the same site.
const LOCK_WAIT: Duration = Duration::from_secs(90);

static EPOCHS: AtomicU64 = AtomicU64::new(1);

struct Entry {
    session: Arc<VaultSession>,
    epoch: u64,
    settings: LockSettings,
    consumers: HashMap<u64, ConsumerKind>,
    locking: bool,
}

type Listener = Arc<dyn Fn(&RegisterEvent) + Send + Sync>;

struct State {
    sites: HashMap<String, Entry>,
    events: VecDeque<RegisterEvent>,
    event_seq: u64,
    next_id: u64,
    participants: Vec<(u64, Arc<dyn LockParticipant>)>,
    listeners: Vec<Listener>,
    screen_on: bool,
    background_since: Option<Instant>,
}

pub(crate) struct SessionRegister {
    options: RegisterOptions,
    state: Mutex<State>,
    changed: Condvar,
    signals: Mutex<Option<Box<dyn SignalControl>>>,
    started: AtomicBool,
    me: Weak<SessionRegister>,
}

/// Holds a consumer; dropping it releases the consumer.
pub(crate) struct ConsumerGuard {
    register: Weak<SessionRegister>,
    site: String,
    epoch: u64,
    id: u64,
}

impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        if let Some(register) = self.register.upgrade() {
            let mut state = register.lock_state();
            if let Some(entry) = state.sites.get_mut(&self.site) {
                if entry.epoch == self.epoch {
                    entry.consumers.remove(&self.id);
                }
            }
        }
    }
}

impl SessionRegister {
    pub(crate) fn new(options: RegisterOptions) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            options,
            state: Mutex::new(State {
                sites: HashMap::new(),
                events: VecDeque::new(),
                event_seq: 0,
                next_id: 0,
                participants: Vec::new(),
                listeners: Vec::new(),
                screen_on: true,
                background_since: None,
            }),
            changed: Condvar::new(),
            signals: Mutex::new(None),
            started: AtomicBool::new(false),
            me: me.clone(),
        })
    }

    fn lock_state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits until no lock of `site` runs (at most `LOCK_WAIT`).
    fn settled<'a>(
        &'a self,
        mut state: MutexGuard<'a, State>,
        site: &str,
    ) -> MutexGuard<'a, State> {
        let until = Instant::now() + LOCK_WAIT;
        while state.sites.get(site).is_some_and(|entry| entry.locking) {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        state
    }

    /// The unlocked session of `site` and its epoch.
    pub(crate) fn session(&self, site: &str) -> Option<(Arc<VaultSession>, u64)> {
        let state = self.lock_state();
        state
            .sites
            .get(site)
            .filter(|entry| !entry.locking && !entry.session.is_locked())
            .map(|entry| (Arc::clone(&entry.session), entry.epoch))
    }

    /// Makes `session` the unlocked session of `record`. A site that is
    /// unlocked already keeps its session; the offered one is dropped by the
    /// caller, which wipes its key.
    pub(crate) fn install(&self, record: &SiteRecord, session: Arc<VaultSession>) -> Installed {
        let epoch = {
            let mut state = self.settled(self.lock_state(), &record.id);
            if let Some(entry) = state.sites.get(&record.id) {
                return Installed {
                    epoch: entry.epoch,
                    already: true,
                };
            }
            let epoch = EPOCHS.fetch_add(1, Ordering::Relaxed);
            let entry = Entry {
                session: Arc::clone(&session),
                epoch,
                settings: record.lock,
                consumers: HashMap::new(),
                locking: false,
            };
            state.sites.insert(record.id.clone(), entry);
            epoch
        };
        if self.options.publish {
            publish_session(session);
        }
        self.emit(&record.id, SiteState::Unlocked, None, epoch);
        self.start_background();
        self.rearm();
        self.changed.notify_all();
        Installed {
            epoch,
            already: false,
        }
    }

    /// Locks `site`: the participants first, then the key is wiped.
    /// `Ok(false)` when it was not unlocked.
    pub(crate) fn lock(&self, site: &str, request: LockRequest) -> Result<bool, LockError> {
        let (session, epoch) = {
            let mut state = self.settled(self.lock_state(), site);
            match state.sites.get_mut(site) {
                Some(entry) if !entry.locking => {
                    entry.locking = true;
                    (Arc::clone(&entry.session), entry.epoch)
                }
                _ => return Ok(false),
            }
        };
        self.emit(site, SiteState::Locking, Some(request.reason), epoch);
        let participants: Vec<Arc<dyn LockParticipant>> = self
            .lock_state()
            .participants
            .iter()
            .map(|(_, participant)| Arc::clone(participant))
            .collect();
        for participant in participants {
            if let Err(unsaved) = participant.prepare_lock(site, &request) {
                if !request.reason.forced() && !request.keep_unsaved {
                    if let Some(entry) = self.lock_state().sites.get_mut(site) {
                        entry.locking = false;
                    }
                    self.changed.notify_all();
                    self.emit(site, SiteState::Unlocked, None, epoch);
                    return Err(LockError::Unsaved(unsaved));
                }
            }
        }
        self.lock_state().sites.remove(site);
        if self.options.publish {
            withdraw_session(site);
        }
        // Wipes the masterkey and tells the lock listeners (directory caches).
        session.lock();
        self.changed.notify_all();
        self.emit(site, SiteState::Locked, Some(request.reason), epoch);
        self.rearm();
        Ok(true)
    }

    /// Locks every unlocked site whose settings are `selected`; returns the
    /// sites it locked. Forced: unsaved work stays sealed.
    pub(crate) fn lock_where(
        &self,
        reason: LockReason,
        selected: impl Fn(&LockSettings) -> bool,
    ) -> Vec<String> {
        let sites: Vec<String> = self
            .lock_state()
            .sites
            .iter()
            .filter(|(_, entry)| selected(&entry.settings))
            .map(|(site, _)| site.clone())
            .collect();
        sites
            .into_iter()
            .filter(|site| matches!(self.lock(site, LockRequest::new(reason, true)), Ok(true)))
            .collect()
    }

    /// „Alle sperren“ (`se vault lock --all`): stops at the first site whose
    /// unsaved changes could not be published.
    pub(crate) fn lock_all_explicit(&self, keep_unsaved: bool) -> Result<Vec<String>, LockError> {
        let mut sites: Vec<String> = self.lock_state().sites.keys().cloned().collect();
        sites.sort();
        let mut locked = Vec::new();
        for site in sites {
            if self.lock(&site, LockRequest::new(LockReason::Explicit, keep_unsaved))? {
                locked.push(site);
            }
        }
        Ok(locked)
    }

    /// Registers a consumer of the unlocked `site`; `None` when locked.
    pub(crate) fn consume(&self, site: &str, kind: ConsumerKind) -> Option<ConsumerGuard> {
        let mut state = self.lock_state();
        state.next_id += 1;
        let id = state.next_id;
        let entry = state.sites.get_mut(site).filter(|entry| !entry.locking)?;
        entry.consumers.insert(id, kind);
        Some(ConsumerGuard {
            register: self.me.clone(),
            site: site.to_string(),
            epoch: entry.epoch,
            id,
        })
    }

    /// A drive of `site` ended (`MountTerminated`, S2-V6): its consumer goes;
    /// unless it ended because of a lock, the vault locks when nothing else
    /// holds the session (C3.3). Returns whether it locked.
    pub(crate) fn mount_ended(&self, site: &str, mount_id: &str, by_lock: bool) -> bool {
        let lock_now = {
            let mut state = self.lock_state();
            let Some(entry) = state.sites.get_mut(site) else {
                return false;
            };
            let drive = ConsumerKind::Drive(mount_id.to_string());
            entry.consumers.retain(|_, kind| *kind != drive);
            !by_lock && !entry.locking && entry.consumers.is_empty()
        };
        lock_now
            && matches!(
                self.lock(site, LockRequest::new(LockReason::MountEnd, true)),
                Ok(true)
            )
    }

    /// New settings of the unlocked sites among `records` („Register lädt
    /// neu“, C7.1 T14).
    pub(crate) fn reload_settings(&self, records: &[SiteRecord]) {
        {
            let mut state = self.lock_state();
            for record in records {
                if let Some(entry) = state.sites.get_mut(&record.id) {
                    entry.settings = record.lock;
                }
            }
        }
        self.rearm();
        self.changed.notify_all();
    }

    /// Every entered site of `records` with its state (`Vault::Status`).
    pub(crate) fn status(&self, records: &[SiteRecord]) -> Vec<SiteStatus> {
        let state = self.lock_state();
        records
            .iter()
            .map(|record| {
                let entry = state.sites.get(&record.id);
                SiteStatus {
                    record: record.clone(),
                    state: match entry {
                        Some(entry) if entry.locking => SiteState::Locking,
                        Some(_) => SiteState::Unlocked,
                        None => SiteState::Locked,
                    },
                    epoch: entry.map(|entry| entry.epoch),
                    consumers: entry.map_or(0, |entry| entry.consumers.len()),
                }
            })
            .collect()
    }

    /// The newest events (at most `EVENT_LOG`) and the number of the last.
    pub(crate) fn events(&self) -> (u64, Vec<RegisterEvent>) {
        let state = self.lock_state();
        (state.event_seq, state.events.iter().cloned().collect())
    }

    /// Calls `listener` for every later event (Android `vault` events).
    pub(crate) fn subscribe(&self, listener: Listener) {
        self.lock_state().listeners.push(listener);
    }

    /// Adds a lock participant; the returned ID removes it again.
    pub(crate) fn add_participant(&self, participant: Arc<dyn LockParticipant>) -> u64 {
        let mut state = self.lock_state();
        state.next_id += 1;
        let id = state.next_id;
        state.participants.push((id, participant));
        id
    }

    pub(crate) fn remove_participant(&self, id: u64) {
        self.lock_state()
            .participants
            .retain(|(known, _)| *known != id);
    }

    fn emit(&self, site: &str, to: SiteState, reason: Option<LockReason>, epoch: u64) {
        let (event, listeners) = {
            let mut state = self.lock_state();
            state.event_seq += 1;
            let event = RegisterEvent {
                seq: state.event_seq,
                site: site.to_string(),
                state: to,
                reason,
                epoch,
            };
            state.events.push_back(event.clone());
            while state.events.len() > EVENT_LOG {
                state.events.pop_front();
            }
            (event, state.listeners.clone())
        };
        for listener in listeners {
            listener(&event);
        }
    }
}

/// The register of this process.
pub(crate) fn register() -> &'static Arc<SessionRegister> {
    static REGISTER: OnceLock<Arc<SessionRegister>> = OnceLock::new();
    REGISTER.get_or_init(|| {
        SessionRegister::new(RegisterOptions {
            publish: true,
            platform: super::LOCK_PLATFORM,
            background: true,
        })
    })
}

/// Whether this process holds the vault keys of the device: the worker on
/// the desktop, the app process on Android. Other processes (desktop GUI,
/// CLI, MCP) reach the worker over IPC.
pub(crate) fn hosts_register() -> bool {
    crate::daemon::hosts_vault_register()
}

/// Screen and visibility of the Android app (`sys.hostState`, C3.4).
pub(crate) fn host_visibility(foreground: bool, screen_on: bool) {
    register().set_host(foreground, screen_on, Instant::now());
}

/// The error frame text of the worker's vault connections (`Vault::Open`)
/// for a locked vault: `SE_VAULT_V1 locked <site> <text>` (C3.2); `None`
/// for every other error, which keeps its text.
pub(crate) fn wire_error_text(error: &std::io::Error) -> Option<String> {
    let site = crate::vault::core::locked_site(error)?;
    Some(crate::agent_proto::vault_error_message(
        crate::agent_proto::VAULT_LOCKED_WIRE,
        site,
        &error.to_string(),
    ))
}

/// The worker ends: every vault locks (C3.3 „Ende des Workers“), so no key
/// stays in the memory of the ending process. Android keeps its register in
/// the app process, which outlives the embedded worker.
pub(crate) fn lock_all_for_exit() {
    if super::LOCK_PLATFORM == LockPlatform::Desktop {
        register().lock_where(LockReason::WorkerEnd, |_| true);
    }
}
