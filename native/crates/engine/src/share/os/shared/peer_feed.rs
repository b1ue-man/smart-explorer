//! Viewer side of the device feed (design 5.2, E13, user rule r5): the
//! background service keeps one metadata session to every known device with
//! a present relation (design 2.2) and subscribes to its window families,
//! shared services and RDP short state; surfaces read
//! `ShareWorkerSnapshot.peer_feeds`. No timer of its own (A4, A5 of plan
//! 2026-11-06-akku-rechte, `peer_feed_restart.rs`): a refused feed starts
//! again on a change of this device's relation to that device, a return to
//! the app or after its refusal backoff; a lost one at once on an arrival of
//! that device or this service's server connection, otherwise after its
//! backoff (which every look of `observe` checks; a host that ended the feed
//! as idle is no arrival while it rests). In
//! low-power operation (app not visible) no feed holds a connection unless
//! a mapped service of its device is in use; the paused and lost feeds
//! start again when the app is visible (`peer_feed_power.rs`). A probe
//! handshake learns the rights first, so the background never makes the
//! host ask its user („Nachfragen“ is reported instead).
use crate::rdp::core::RdpFeedSummary;
use crate::screen::core::family_protocol::FamilyListing;
use crate::share::auto_forward::{self, MappedService};
use crate::share::loopback_services::LoopbackService;
use crate::share::peer_select::present_relation;
use crate::share::{PeerOpenTarget, ShareEvent, ShareProfiles, ShareService};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;
use tokio::sync::watch;

/// The known devices and the events that report an arrival.
#[path = "peer_feed_devices.rs"]
mod devices;
/// Handshake, subscription and update loop of one feed session.
#[path = "peer_feed_link.rs"]
mod link;
/// Restarts after visibility returns.
#[path = "peer_feed_power.rs"]
mod power;
/// The relation to one device, without runtime data.
#[path = "peer_feed_relation.rs"]
mod relation;
/// When a session starts, stays, pauses or waits.
#[path = "peer_feed_restart.rs"]
mod restart;

use devices::{arrived_devices, identities, tunnel_in_use, Identity};
use relation::Relation;
use restart::{Facts, Phase, Step};

/// Share of the 2 MiB IPC line for the feeds of all devices.
const MAX_SNAPSHOT_BYTES: usize = 768 * 1024;
const TOO_LARGE: &str = "Zu groß für den Statuskanal; `se screen families` ruft einzeln ab";
const PAUSED: &str =
    "Gerätefeed ruht, solange diese App im Hintergrund ist; er läuft wieder, sobald sie sichtbar ist";

/// The feed state of one device, as every surface shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeerFeedView {
    pub device_id: String,
    pub public_key: String,
    pub name: String,
    pub live: bool,
    pub families: Option<FamilyListing>,
    pub families_unavailable: Option<String>,
    pub services: Vec<LoopbackService>,
    pub services_unavailable: Option<String>,
    /// How this device reaches each service (design 4.3).
    pub mapped: Vec<MappedService>,
    pub rdp: Option<RdpFeedSummary>,
    /// Why there is no feed (older version, rights, connection).
    pub unavailable: Option<String>,
    /// The feed ended by a lost connection (not a refusal): it starts again
    /// after its backoff or on an event of that device (review B7, A5).
    #[serde(skip)]
    pub retry_on_network: bool,
    /// The host ended the quiet feed as idle in low power (`peer_feed_restart.rs`).
    #[serde(skip)]
    pub host_rested: bool,
}

struct Session {
    service: ShareService,
    target: PeerOpenTarget,
    generation: u64,
    stop: watch::Sender<bool>,
    /// `None`: the thread did not start (shown in the view) or the session
    /// is paused.
    thread: Option<std::thread::JoinHandle<()>>,
    view: Arc<Mutex<PeerFeedView>>,
    /// Ended by this device's low-power operation.
    paused: bool,
    /// The relation this session last ran or ended under.
    relation: Relation,
    /// Lost attempts in a row before this one that never subscribed.
    failures: u32,
    /// The subscription was reached once.
    subscribed: Arc<AtomicBool>,
    /// When a lost session may start again; set when `observe` first sees
    /// it lost.
    retry_at: Option<Instant>,
}

impl Session {
    fn phase(&self) -> Phase {
        if self.paused {
            return Phase::Paused;
        }
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            return Phase::Running;
        }
        let view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        if self.thread.is_none() || view.retry_on_network {
            Phase::Lost
        } else {
            Phase::Refused
        }
    }

    /// Lost attempts in a row, this one included unless it subscribed.
    fn lost_in_a_row(&self) -> u32 {
        if self.subscribed.load(Ordering::Acquire) {
            0
        } else {
            self.failures.saturating_add(1)
        }
    }

    /// Whether the wait of a lost or refused session (from its first look) is over.
    fn backoff_over(&mut self, phase: Phase, now: Instant) -> bool {
        let before = self.lost_in_a_row().saturating_sub(1);
        let wait = match phase {
            Phase::Refused => restart::refused_backoff(before),
            _ if self.host_rested() => restart::RESTED_RETRY,
            _ => restart::backoff(before),
        };
        now >= *self.retry_at.get_or_insert(now + wait)
    }

    /// The host ended this session as idle (see `PeerFeedView::host_rested`).
    fn host_rested(&self) -> bool {
        let view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        view.host_rested
    }

    fn identity(&self) -> Identity {
        let view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        (
            view.device_id.clone(),
            view.public_key.clone(),
            view.name.clone(),
        )
    }
}

#[derive(Default)]
struct Feeds {
    sessions: BTreeMap<String, Session>,
    generation: u64,
}

static FEEDS: OnceLock<Mutex<Feeds>> = OnceLock::new();

fn feeds() -> std::sync::MutexGuard<'static, Feeds> {
    FEEDS
        .get_or_init(|| Mutex::new(Feeds::default()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// After every event drain and profile reload of the background service:
/// starts, keeps, pauses or ends the feed session of every known device.
pub fn observe(service: Option<&ShareService>, profiles: &ShareProfiles, events: &[ShareEvent]) {
    let Some(service) = service else {
        stop_all();
        return;
    };
    let low_power = crate::share::power::low_power();
    let network = events
        .iter()
        .any(|event| matches!(event, ShareEvent::ServerConnected));
    let arrived = arrived_devices(profiles, events);
    let known = identities(profiles);
    let now = Instant::now();
    let mut state = feeds();
    let gone: Vec<String> = state
        .sessions
        .keys()
        .filter(|key| !known.contains_key(*key))
        .cloned()
        .collect();
    for key in gone {
        end_session(&mut state, &key);
    }
    for (key, identity) in known {
        let Some(target) = present_relation(profiles, &identity.0, &identity.1) else {
            end_session(&mut state, &key);
            continue;
        };
        let relation = Relation::of(profiles, &identity.0, &identity.1);
        let (step, failures) = match state.sessions.get_mut(&key) {
            None => (restart::first(low_power), 0),
            Some(session) => {
                let phase = session.phase();
                // A resting host still sends presence: no arrival for its feed.
                let device_event = network
                    || (!(phase == Phase::Lost && session.host_rested())
                        && arrived.contains(&identity.0));
                let facts = Facts {
                    phase,
                    same: Arc::ptr_eq(&session.service.iroh, &service.iroh)
                        && session.target == target,
                    low_power,
                    tunnel_in_use: phase == Phase::Running && low_power && tunnel_in_use(&identity),
                    relation_changed: session.relation != relation,
                    device_event,
                    backoff_over: matches!(phase, Phase::Lost | Phase::Refused)
                        && session.backoff_over(phase, now),
                };
                let step = restart::step(&facts);
                if step == Step::Keep && matches!(phase, Phase::Running | Phase::Paused) {
                    session.relation = relation.clone();
                }
                // Only a retry the backoff allowed keeps counting; an event
                // of the device or a new relation starts from the beginning.
                let backoff_only = facts.same
                    && !facts.relation_changed
                    && (phase == Phase::Refused || (phase == Phase::Lost && !device_event));
                let failures = if backoff_only {
                    session.lost_in_a_row()
                } else {
                    0
                };
                (step, failures)
            }
        };
        match step {
            Step::Keep => {}
            Step::Start => {
                end_session(&mut state, &key);
                start_session(
                    &mut state, service, key, identity, target, relation, failures,
                );
            }
            Step::Pause => {
                end_session(&mut state, &key);
                pause_session(&mut state, service, key, identity, target, relation);
            }
        }
    }
    // Low-power operation may have ended while this look paused a feed; the
    // visibility thread then found nothing to start.
    if low_power && !crate::share::power::low_power() {
        resume_locked(&mut state, false);
    }
}

/// The app became visible: every paused, lost and refused feed starts at
/// once with the service and target it last had; the next `observe`
/// corrects both.
fn resume() {
    if !crate::share::power::low_power() {
        resume_locked(&mut feeds(), true);
    }
}

/// Starts the paused sessions and, with `lost`, the lost and refused ones.
fn resume_locked(state: &mut Feeds, lost: bool) {
    let waiting: Vec<String> = state
        .sessions
        .iter()
        .filter(|(_, session)| match session.phase() {
            Phase::Paused => true,
            Phase::Lost | Phase::Refused => lost,
            Phase::Running => false,
        })
        .map(|(key, _)| key.clone())
        .collect();
    for key in waiting {
        let Some(session) = state.sessions.get(&key) else {
            continue;
        };
        let (service, target) = (session.service.clone(), session.target.clone());
        let (identity, relation) = (session.identity(), session.relation.clone());
        end_session(state, &key);
        start_session(state, &service, key, identity, target, relation, 0);
    }
}

/// Ends every feed and every mapping (share service stopped).
pub fn stop_all() {
    let mut state = feeds();
    let keys: Vec<String> = state.sessions.keys().cloned().collect();
    for key in keys {
        end_session(&mut state, &key);
    }
    auto_forward::remove_all();
}

fn end_session(state: &mut Feeds, key: &str) {
    if let Some(session) = state.sessions.remove(key) {
        session.stop.send_replace(true);
        let view = session.view.lock().unwrap_or_else(PoisonError::into_inner);
        auto_forward::remove(&view.device_id, &view.public_key);
    }
}

fn new_view(identity: Identity) -> Arc<Mutex<PeerFeedView>> {
    let (device_id, public_key, name) = identity;
    Arc::new(Mutex::new(PeerFeedView {
        device_id,
        public_key,
        name,
        ..PeerFeedView::default()
    }))
}

/// A paused session holds no thread and no connection; its view keeps
/// nothing of the device (threat T-W1) and says why.
fn pause_session(
    state: &mut Feeds,
    service: &ShareService,
    key: String,
    identity: Identity,
    target: PeerOpenTarget,
    relation: Relation,
) {
    state.generation += 1;
    let view = new_view(identity);
    view.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .unavailable = Some(PAUSED.into());
    let session = Session {
        service: service.clone(),
        target,
        generation: state.generation,
        stop: watch::channel(false).0,
        thread: None,
        view,
        paused: true,
        relation,
        failures: 0,
        subscribed: Arc::new(AtomicBool::new(false)),
        retry_at: None,
    };
    state.sessions.insert(key, session);
    power::follow_visibility();
}

fn start_session(
    state: &mut Feeds,
    service: &ShareService,
    key: String,
    identity: Identity,
    target: PeerOpenTarget,
    relation: Relation,
    failures: u32,
) {
    state.generation += 1;
    let generation = state.generation;
    let view = new_view(identity);
    let subscribed = Arc::new(AtomicBool::new(false));
    let (stop, stopped) = watch::channel(false);
    let context = Context {
        service: service.clone(),
        target: target.clone(),
        key: key.clone(),
        generation,
        view: view.clone(),
        subscribed: subscribed.clone(),
    };
    let spawned = std::thread::Builder::new()
        .name("share-peer-feed".into())
        .spawn(move || run(context, stopped));
    // A failed start is shown and retried like a lost session.
    let thread = spawned
        .map_err(|error| {
            let mut shown = view.lock().unwrap_or_else(PoisonError::into_inner);
            shown.unavailable = Some(format!("Gerätefeed nicht gestartet: {error}"));
        })
        .ok();
    let session = Session {
        service: service.clone(),
        target,
        generation,
        stop,
        thread,
        view,
        paused: false,
        relation,
        failures,
        subscribed,
        retry_at: None,
    };
    state.sessions.insert(key, session);
}

/// Every device's feed for `ShareWorkerSnapshot.peer_feeds`; an oversized
/// list is replaced by its reason.
pub fn views() -> Vec<PeerFeedView> {
    let state = feeds();
    let mut all: Vec<PeerFeedView> = state
        .sessions
        .values()
        .map(|session| {
            let mut view = session
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            view.mapped = auto_forward::views(&view.device_id, &view.public_key);
            view
        })
        .collect();
    drop(state);
    let size = |view: &PeerFeedView| serde_json::to_vec(view).map_or(usize::MAX, |b| b.len());
    let mut used = 0usize;
    for view in &mut all {
        if used.saturating_add(size(view)) > MAX_SNAPSHOT_BYTES && view.families.take().is_some() {
            view.families_unavailable = Some(TOO_LARGE.into());
        }
        if used.saturating_add(size(view)) > MAX_SNAPSHOT_BYTES {
            view.services.clear();
            view.mapped.clear();
            view.services_unavailable = Some(TOO_LARGE.into());
        }
        used = used.saturating_add(size(view));
    }
    all
}

struct Context {
    service: ShareService,
    target: PeerOpenTarget,
    key: String,
    generation: u64,
    view: Arc<Mutex<PeerFeedView>>,
    /// Set once the subscription stands (`link::subscribed`).
    subscribed: Arc<AtomicBool>,
}
impl Context {
    fn current_in(&self, state: &Feeds) -> bool {
        let session = state.sessions.get(&self.key);
        session.is_some_and(|session| session.generation == self.generation)
    }
    fn show(&self, change: impl FnOnce(&mut PeerFeedView)) {
        change(&mut self.view.lock().unwrap_or_else(PoisonError::into_inner));
    }
    /// Applies `services` only while this session is current, so an ended
    /// session never re-creates listeners.
    fn map_services(&self, services: &[LoopbackService]) {
        let state = feeds();
        if !self.current_in(&state) {
            return;
        }
        let (device_id, public_key) = {
            let view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
            (view.device_id.clone(), view.public_key.clone())
        };
        let applied = auto_forward::apply(
            &self.service,
            &device_id,
            &public_key,
            &self.target,
            services,
        );
        drop(state);
        if let Err(error) = applied {
            self.show(|view| view.services_unavailable = Some(error));
        }
    }
}

fn run(context: Context, stopped: watch::Receiver<bool>) {
    // `None`: stopped by `observe`, which also removed the mappings.
    if let Some(ended) = link::feed(&context, stopped) {
        context.map_services(&[]);
        // An ended feed keeps nothing of the device (threat T-W1 „Feed endet
        // mit dem Recht“): no windows, services or RDP state stay visible.
        context.show(|view| {
            view.live = false;
            view.families = None;
            view.families_unavailable = None;
            view.services.clear();
            view.services_unavailable = None;
            view.rdp = None;
            view.unavailable = Some(ended.reason);
            view.retry_on_network = ended.connection_lost;
            view.host_rested = ended.host_rested;
        });
    }
}
