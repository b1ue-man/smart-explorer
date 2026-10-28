//! Viewer side of the device feed (design 5.2, E13, user rule r5): the
//! background service keeps one metadata session to every known device with
//! a present relation (design 2.2) and subscribes to its window families,
//! shared services and RDP short state; surfaces read
//! `ShareWorkerSnapshot.peer_feeds`. No timer retries: an ended session
//! starts again only on a presence event of that device or a new profile
//! revision. A probe handshake learns the rights first, so the background
//! never makes the host ask its user („Nachfragen“ is reported instead).
use crate::rdp::core::RdpFeedSummary;
use crate::screen::core::family_protocol::{FamilyListing, FAMILY_PROTOCOL};
use crate::screen::core::feed_host::decode_feed_update;
use crate::screen::core::negotiate::CodecOffer;
use crate::screen::core::viewer_client::{refusal_text, ViewerRequest};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire::{self, FAMILY_FEED_VERSION, FEED_UPDATE, MAX_WINDOWS_BYTES};
use crate::screen::core::wire_control::{send_subscribe_feed, ScreenWireError};
use crate::screen::core::wire_hello::{
    recv_hello_reply, recv_server_hello, send_client_hello, HelloReply, ScreenHelloOk, SessionMode,
};
use crate::share::access_grant::CapabilityDecision;
use crate::share::auto_forward::{self, MappedService};
use crate::share::loopback_services::LoopbackService;
use crate::share::node_screen::ScreenLink;
use crate::share::peer_select::present_relation;
use crate::share::{DirectContact, PeerOpenTarget, ShareEvent, ShareProfiles, ShareService};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;
use tokio::sync::watch;

/// Per handshake step on the active route (design 7: kept).
const HANDSHAKE: Duration = Duration::from_secs(20);
/// Share of the 2 MiB IPC line for the feeds of all devices.
const MAX_SNAPSHOT_BYTES: usize = 768 * 1024;
const OLDER_VERSION: &str = "Gerät hat eine ältere Version";
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
    if let Some(reason) = feed(&context, stopped) {
        context.map_services(&[]);
        context.show(|view| {
            view.live = false;
            view.unavailable = Some(reason);
        });
    }
}

fn viewer_request(mode: SessionMode) -> ViewerRequest {
    ViewerRequest {
        mode,
        source: (mode != SessionMode::Probe).then_some(SourceId::Synthetic),
        decoders: vec![CodecOffer::tiles()],
        display_size: None,
        max_fps: 1,
        frame_capabilities: None,
    }
}

/// One raw handshake on a fresh connection: the probe (rights, no question
/// to the host's user) or the metadata channel.
async fn handshake(
    link: ScreenLink,
    mode: SessionMode,
) -> Result<(ScreenHelloOk, iroh::endpoint::Connection, Streams), String> {
    let connection = link.connection.clone();
    let (mut send, mut recv) =
        crate::share::route_protocol::run_with_route(&connection, HANDSHAKE, None, async {
            connection.accept_bi().await.map_err(io::Error::other)
        })
        .await
        .map_err(|error| error.to_string())?;
    let server = crate::share::route_protocol::run_with_route(
        &connection,
        HANDSHAKE,
        None,
        recv_server_hello(&mut recv),
    )
    .await
    .map_err(|error| error.to_string())?;
    if !wire::supports_version(server.protocol_version) {
        return Err(OLDER_VERSION.into());
    }
    if mode != SessionMode::Probe && server.family_feed_version != Some(FAMILY_FEED_VERSION) {
        return Err(OLDER_VERSION.into());
    }
    let mut hello = link
        .client_hello(&server, &viewer_request(mode))
        .map_err(|error| error.to_string())?;
    // Outside the signed transcript, as in `viewer_client::handshake_for`.
    hello.family_version = server.family_version.filter(|v| *v == FAMILY_PROTOCOL);
    hello.icon_version = None;
    hello.metadata_only = mode != SessionMode::Probe && hello.family_version.is_some();
    hello.frame_capabilities = None;
    connection.set_max_concurrent_uni_streams(iroh::endpoint::VarInt::from_u32(0));
    send_client_hello(&mut send, &hello)
        .await
        .map_err(|error| error.to_string())?;
    let reply = crate::share::route_protocol::run_with_route(
        &connection,
        HANDSHAKE,
        None,
        recv_hello_reply(&mut recv),
    )
    .await
    .map_err(|error| error.to_string())?;
    match reply {
        HelloReply::Accepted(accepted) => Ok((accepted, connection, (send, recv))),
        HelloReply::Refused(refused) => Err(refusal_text(&refused.code)),
        HelloReply::Pending(_) => {
            // Never a question from the background: withdraw at once.
            connection.close(iroh::endpoint::VarInt::from_u32(0), b"feed does not ask");
            Err(ASKS.into())
        }
    }
}

type Streams = (iroh::endpoint::SendStream, iroh::endpoint::RecvStream);
const ASKS: &str =
    "„Bildschirm ansehen“ steht auf diesem Gerät auf „Nachfragen“; Fenster und Dienste erscheinen mit „Erlauben“";

/// A new screen connection to the device (blocking).
fn link(context: &Context) -> Result<ScreenLink, String> {
    context
        .service
        .screen_link(&context.target)
        .map_err(|error| error.detail())
}

/// Probe, subscribe, apply every update; the reason the feed ended, `None`
/// when `observe` stopped it.
fn feed(context: &Context, mut stopped: watch::Receiver<bool>) -> Option<String> {
    let runtime = context.service.screen_runtime();
    let probe = match link(context) {
        Ok(link) => runtime.block_on(handshake(link, SessionMode::Probe)),
        Err(error) => return Some(error),
    };
    let (probe, connection, _) = match probe {
        Ok(opened) => opened,
        Err(error) => return Some(error),
    };
    connection.close(iroh::endpoint::VarInt::from_u32(0), b"probe done");
    match probe.authorization.view {
        CapabilityDecision::Allow => {}
        CapabilityDecision::Ask => return Some(ASKS.into()),
        _ => return Some(refusal_text("permission_denied")),
    }
    if *stopped.borrow() {
        return None;
    }
    let link = match link(context) {
        Ok(link) => link,
        Err(error) => return Some(error),
    };
    runtime.block_on(async {
        tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => None,
            ended = subscribed(context, link) => Some(ended),
        }
    })
}

async fn subscribed(context: &Context, link: ScreenLink) -> String {
    let (_, connection, (mut send, mut recv)) = match handshake(link, SessionMode::View).await {
        Ok(opened) => opened,
        Err(error) => return error,
    };
    if let Err(error) = send_subscribe_feed(&mut send).await {
        return format!("Gerätefeed nicht abonniert: {error}");
    }
    context.show(|view| {
        view.live = true;
        view.unavailable = None;
    });
    // No receive limit: QUIC's idle detection (20 s) ends a dead feed.
    let ended = loop {
        let (tag, payload) = match wire::read_frame(&mut recv, MAX_WINDOWS_BYTES).await {
            Ok(frame) => frame,
            Err(error) => break format!("Gerätefeed beendet: {error}"),
        };
        if tag == wire::ERROR {
            let error: Result<ScreenWireError, _> = wire::from_json(&payload);
            break error.map_or_else(|e| e.to_string(), |error| refusal_text(&error.code));
        }
        if tag != FEED_UPDATE {
            continue;
        }
        let update = match decode_feed_update(&payload) {
            Ok(update) => update,
            Err(error) => break format!("Ungültiges Gerätefeed-Update: {error}"),
        };
        let services = update.services.clone().unwrap_or_default();
        context.show(|view| {
            view.families = update.families;
            view.families_unavailable = update.families_unavailable;
            view.services = services.clone();
            view.services_unavailable = update.services_unavailable;
            view.rdp = update.rdp;
        });
        context.map_services(&services);
    };
    connection.close(iroh::endpoint::VarInt::from_u32(0), b"feed ended");
    drop(send);
    ended
}
