//! Viewer side of the device feed (design 5.2, E13, user rule r5): the
//! background service keeps one metadata session to every known device with
//! a present relation (design 2.2) and subscribes to its window families,
//! shared services and RDP short state; surfaces read
//! `ShareWorkerSnapshot.peer_feeds`. No timer retries: an ended session
//! starts again only on a presence event of that device, a new profile
//! revision or, after a lost connection, this service's reconnect. A probe
//! handshake learns the rights first, so the background never makes the
//! host ask its user („Nachfragen“ is reported instead).
use crate::rdp::core::RdpFeedSummary;
use crate::screen::core::family_protocol::FamilyListing;
use crate::share::auto_forward::{self, MappedService};
use crate::share::loopback_services::LoopbackService;
use crate::share::peer_select::present_relation;
use crate::share::{DirectContact, PeerOpenTarget, ShareEvent, ShareProfiles, ShareService};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use tokio::sync::watch;

/// Handshake, subscription and update loop of one feed session.
#[path = "peer_feed_link.rs"]
mod link;

/// Share of the 2 MiB IPC line for the feeds of all devices.
const MAX_SNAPSHOT_BYTES: usize = 768 * 1024;
const TOO_LARGE: &str = "Zu groß für den Statuskanal; `se screen families` ruft einzeln ab";

/// The feed state of one device, as every surface shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PeerFeedView {
    pub(crate) device_id: String,
    pub(crate) public_key: String,
    pub(crate) name: String,
    pub(crate) live: bool,
    pub(crate) families: Option<FamilyListing>,
    pub(crate) families_unavailable: Option<String>,
    pub(crate) services: Vec<LoopbackService>,
    pub(crate) services_unavailable: Option<String>,
    /// How this device reaches each service (design 4.3).
    pub(crate) mapped: Vec<MappedService>,
    pub(crate) rdp: Option<RdpFeedSummary>,
    /// Why there is no feed (older version, rights, connection).
    pub(crate) unavailable: Option<String>,
    /// The feed ended by a lost connection (not a refusal): it starts again
    /// on the next reconnect of this service to the network (review B7).
    #[serde(skip)]
    pub(crate) retry_on_network: bool,
}

struct Session {
    service: ShareService,
    target: PeerOpenTarget,
    generation: u64,
    stop: watch::Sender<bool>,
    /// `None`: the thread did not start (shown in the view).
    thread: Option<std::thread::JoinHandle<()>>,
    view: Arc<Mutex<PeerFeedView>>,
}

#[derive(Default)]
struct Feeds {
    sessions: BTreeMap<String, Session>,
    retry: BTreeSet<String>,
    revision: Option<crate::share::ProfileRevision>,
    generation: u64,
}

static FEEDS: OnceLock<Mutex<Feeds>> = OnceLock::new();

fn feeds() -> std::sync::MutexGuard<'static, Feeds> {
    FEEDS
        .get_or_init(|| Mutex::new(Feeds::default()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Device ID, public key, name.
type Identity = (String, String, String);

fn identities(profiles: &ShareProfiles) -> BTreeMap<String, Identity> {
    let mut known = BTreeMap::new();
    for contact in &profiles.direct_contacts {
        if let (Some(device), Some(key)) = (&contact.remote_device_id, &contact.remote_public_key) {
            let identity = (device.clone(), key.clone(), contact.display_name.clone());
            known.insert(auto_forward::device_key(device, key), identity);
        }
    }
    for room in profiles.rooms.iter().filter(|room| room.auto_join) {
        for member in room.members.iter().filter(|member| !member.blocked) {
            let key = auto_forward::device_key(&member.device_id, &member.public_key);
            known.entry(key).or_insert_with(|| {
                let name = member.device_name.clone();
                (member.device_id.clone(), member.public_key.clone(), name)
            });
        }
    }
    known
}

/// Device IDs named by presence events (design 5.2 retry triggers).
fn present_devices(profiles: &ShareProfiles, events: &[ShareEvent]) -> BTreeSet<String> {
    let contact = |pick: &dyn Fn(&DirectContact) -> bool| {
        profiles
            .direct_contacts
            .iter()
            .find(|contact| pick(contact))
            .and_then(|contact| contact.remote_device_id.clone())
    };
    let mut devices = BTreeSet::new();
    for event in events {
        match event {
            ShareEvent::DirectAvailable { presence, .. }
            | ShareEvent::RoomJoined { presence, .. } => {
                devices.insert(presence.device_id.clone());
            }
            ShareEvent::RoomRoster { members, .. } => {
                devices.extend(members.iter().map(|member| member.device_id.clone()));
            }
            ShareEvent::RoomLeft { device_id, .. } => {
                devices.insert(device_id.clone());
            }
            ShareEvent::DirectOffline { lookup_id } => {
                devices.extend(contact(&|c: &DirectContact| &c.lookup_id == lookup_id));
            }
            ShareEvent::LanPeerSeen { contact_id, .. } | ShareEvent::LanPeerLost { contact_id } => {
                devices.extend(contact(&|c: &DirectContact| &c.id == contact_id));
            }
            _ => {}
        }
    }
    devices
}

/// After every event drain and profile reload of the background service:
/// starts, keeps or ends the feed session of every known device.
pub(crate) fn observe(
    service: Option<&ShareService>,
    profiles: &ShareProfiles,
    events: &[ShareEvent],
) {
    let Some(service) = service else {
        stop_all();
        return;
    };
    let mut state = feeds();
    let revision_changed = state.revision.as_ref() != Some(&profiles.storage_revision);
    state.revision = Some(profiles.storage_revision.clone());
    let moved = present_devices(profiles, events);
    let known = identities(profiles);
    for (key, (device_id, _, _)) in &known {
        if moved.contains(device_id) {
            state.retry.insert(key.clone());
        }
    }
    // Event, not timer: a feed lost with the connection starts again once
    // this service is back on the network.
    if events
        .iter()
        .any(|event| matches!(event, ShareEvent::ServerConnected))
    {
        let lost: Vec<String> = state
            .sessions
            .iter()
            .filter(|(_, session)| {
                let ended = session.thread.as_ref().is_none_or(|t| t.is_finished());
                let view = session.view.lock().unwrap_or_else(PoisonError::into_inner);
                ended && view.retry_on_network
            })
            .map(|(key, _)| key.clone())
            .collect();
        state.retry.extend(lost);
    }
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
        let target = present_relation(profiles, &identity.0, &identity.1);
        let decision = match (&target, state.sessions.get(&key)) {
            (None, Some(_)) => Some(false),
            (None, None) => None,
            (Some(target), Some(session)) => {
                let same =
                    Arc::ptr_eq(&session.service.iroh, &service.iroh) && &session.target == target;
                let running = session.thread.as_ref().is_some_and(|t| !t.is_finished());
                let retry = revision_changed || state.retry.contains(&key);
                (!(same && running) && (running || retry)).then_some(true)
            }
            (Some(_), None) => Some(true),
        };
        match (decision, target) {
            (Some(true), Some(target)) => {
                end_session(&mut state, &key);
                start_session(&mut state, service, key, identity, target);
            }
            (Some(_), _) => end_session(&mut state, &key),
            (None, _) => {}
        }
    }
}

/// Ends every feed and every mapping (share service stopped).
pub(crate) fn stop_all() {
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

fn start_session(
    state: &mut Feeds,
    service: &ShareService,
    key: String,
    identity: Identity,
    target: PeerOpenTarget,
) {
    state.retry.remove(&key);
    state.generation += 1;
    let generation = state.generation;
    let (device_id, public_key, name) = identity;
    let view = Arc::new(Mutex::new(PeerFeedView {
        device_id,
        public_key,
        name,
        ..PeerFeedView::default()
    }));
    let (stop, stopped) = watch::channel(false);
    let context = Context {
        service: service.clone(),
        target: target.clone(),
        key: key.clone(),
        generation,
        view: view.clone(),
    };
    let spawned = std::thread::Builder::new()
        .name("share-peer-feed".into())
        .spawn(move || run(context, stopped));
    // A failed start is shown and retried like an ended session.
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
    };
    state.sessions.insert(key, session);
}

/// Every device's feed for `ShareWorkerSnapshot.peer_feeds`; an oversized
/// list is replaced by its reason.
pub(crate) fn views() -> Vec<PeerFeedView> {
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
        });
    }
}

