//! Device feed of the host (design 5.2, user rule r5 „wenn n fenster offen
//! ist sehen es andere se desktops, egal ob lokal der se desktop an ist“).
//! After `SUBSCRIBE_FEED` on a metadata channel the background service
//! samples once per second — window families, steering-exposed local
//! services, the RDP short state — and sends `FEED_UPDATE` only on change.
//! The second is a sampling rate, not a deadline: neither Windows nor Linux
//! signals new listening sockets, and the window adapter polls anyway.
//! Rights per send: windows need „Bildschirm ansehen: Erlauben“, services
//! and RDP need steering. The 15 s receive limit of the single-request
//! metadata channel does not apply; QUIC's idle detection (20 s) watches it.
use super::family_protocol::FamilyListing;
use super::host_server::HostContext;
use super::session_state::EndReason;
use super::wire::{write_frame, FEED_UPDATE, MAX_WINDOWS_BYTES};
use super::wire_control::{recv_metadata_frame, HostFrame, MetadataFrame, Pong, ViewerFrame};
use super::wire_hello::SessionMode;
use crate::rdp::core::RdpFeedSummary;
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::loopback_services::{LoopbackService, MAX_SERVICES};
use crate::share::relation_proof::ProvenPrincipal;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// Sampling rate of the feed (design 7: a rate, no time limit).
pub(crate) const FEED_RATE: Duration = Duration::from_secs(1);
const MAX_TEXT: usize = 4096;

/// Payload of `FEED_UPDATE` (design 9.4): always the complete state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FeedUpdate {
    pub families: Option<FamilyListing>,
    pub families_unavailable: Option<String>,
    pub services: Option<Vec<LoopbackService>>,
    pub services_unavailable: Option<String>,
    pub rdp: Option<RdpFeedSummary>,
}
impl FeedUpdate {
    /// Bounds of a received update; the viewer refuses anything else.
    pub(crate) fn validate(&self) -> io::Result<()> {
        let invalid = |message: &'static str| io::Error::new(io::ErrorKind::InvalidData, message);
        if let Some(families) = &self.families {
            families.validate()?;
        }
        let oversized = |text: &Option<String>| text.as_ref().is_some_and(|t| t.len() > MAX_TEXT);
        if oversized(&self.families_unavailable) || oversized(&self.services_unavailable) {
            return Err(invalid("device feed text exceeds its bound"));
        }
        if let Some(services) = &self.services {
            if services.len() > MAX_SERVICES || services.iter().any(|s| s.validate().is_err()) {
                return Err(invalid("device feed service list is invalid"));
            }
        }
        if let Some(rdp) = &self.rdp {
            let alias_ok = |alias: &str| crate::rdp::core::PublishedApp::valid_alias(alias);
            let apps = &rdp.remote_apps;
            if apps.len() > 1024 || !apps.iter().all(|app| alias_ok(&app.alias)) {
                return Err(invalid("device feed RDP state is invalid"));
            }
        }
        Ok(())
    }
}

/// The JSON of one update within the `FEED_UPDATE` budget. A window list
/// above 1 MiB is replaced by its reason instead of being cut.
pub(crate) fn encode_feed_update(update: &FeedUpdate) -> io::Result<Vec<u8>> {
    let bytes = super::wire::to_json(update)?;
    if bytes.len() <= MAX_WINDOWS_BYTES {
        return Ok(bytes);
    }
    let mut smaller = update.clone();
    smaller.families = None;
    smaller.families_unavailable = Some("Fensterliste überschreitet 1 MiB".into());
    let bytes = super::wire::to_json(&smaller)?;
    if bytes.len() <= MAX_WINDOWS_BYTES {
        return Ok(bytes);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "device feed update exceeds its frame budget",
    ))
}

/// Viewer side: one received `FEED_UPDATE` payload, validated
/// (`peer_feed_link::subscribed`).
pub(crate) fn decode_feed_update(payload: &[u8]) -> io::Result<FeedUpdate> {
    let update: FeedUpdate = super::wire::from_json(payload)?;
    update.validate()?;
    Ok(update)
}

fn allowed(context: &HostContext, principal: &ProvenPrincipal, capability: Capability) -> bool {
    let now = crate::share::core_now_secs();
    context
        .authority
        .capability(principal, capability, now)
        .is_some_and(|state| state.decision == CapabilityDecision::Allow)
}

type Listing = Result<Vec<LoopbackService>, String>;

/// One service listing per rate for every subscribed viewer of this host.
static SERVICES: Mutex<Option<(Instant, Listing)>> = Mutex::new(None);

async fn services() -> Listing {
    if let Ok(cached) = SERVICES.lock() {
        if let Some((at, listing)) = cached.as_ref() {
            if at.elapsed() < FEED_RATE {
                return listing.clone();
            }
        }
    }
    let listing = tokio::task::spawn_blocking(crate::share::tcp_sockets::local_services)
        .await
        .unwrap_or_else(|error| Err(format!("service listing ended: {error}")));
    if let Ok(mut cached) = SERVICES.lock() {
        *cached = Some((Instant::now(), listing.clone()));
    }
    listing
}

/// One complete feed state for `principal`, under the rights of this moment.
async fn sample(
    context: &HostContext,
    principal: &ProvenPrincipal,
    connection: &Connection,
    stop: &watch::Receiver<Option<EndReason>>,
    families_supported: bool,
) -> io::Result<FeedUpdate> {
    let mut update = FeedUpdate::default();
    if !families_supported {
        update.families_unavailable = Some("Fensterfamilien bietet dieses Gerät nicht an".into());
    } else if allowed(context, principal, Capability::ScreenView) {
        let mut job = super::host_metadata_jobs::spawn(
            context,
            principal,
            &[Capability::ScreenView],
            super::family_listing::list,
        );
        let until = super::host_metadata::until(connection, Some(stop.clone()), job.result());
        match until.await {
            Ok(listing) => update.families = Some(listing),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => update.families_unavailable = Some(error.to_string()),
        }
    }
    if allowed(context, principal, Capability::ScreenControl) {
        match services().await {
            Ok(services) => update.services = Some(services),
            Err(reason) => update.services_unavailable = Some(reason),
        }
        update.rdp = crate::rdp::core::host_summary();
    }
    Ok(update)
}

/// Serves a subscribed metadata channel until the viewer leaves, the
/// session stops or a right ends. `recv` moves into a reader task so the
/// sampling rate never cancels a half-read frame.
pub(crate) async fn run(
    context: &HostContext,
    principal: &ProvenPrincipal,
    connection: &Connection,
    send: &mut SendStream,
    recv: RecvStream,
    mut stop: watch::Receiver<Option<EndReason>>,
    families_supported: bool,
) -> io::Result<()> {
    let (frames, mut incoming) = tokio::sync::mpsc::channel(4);
    let reader = tokio::spawn(async move {
        let mut recv = recv;
        loop {
            let frame = recv_metadata_frame(&mut recv).await;
            let end = frame.is_err();
            if frames.send(frame).await.is_err() || end {
                break;
            }
        }
    });
    let _reader = AbortOnDrop(reader);
    let mut tick = tokio::time::interval(FEED_RATE);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last: Option<Vec<u8>> = None;
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => return Ok(()),
            frame = incoming.recv() => match frame {
                Some(Ok(MetadataFrame::Viewer(ViewerFrame::Ping(ping)))) => {
                    let now = super::gop_stream::now_us();
                    let pong = HostFrame::Pong(Pong::answer(&ping, now, now));
                    let sent = super::host_metadata::send(
                        context,
                        principal,
                        SessionMode::View,
                        connection,
                        send,
                        &pong,
                        stop.clone(),
                    );
                    sent.await?;
                }
                Some(Ok(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a subscribed device feed accepts only pings",
                    ));
                }
                Some(Err(error)) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Some(Err(error)) => return Err(error),
                None => return Ok(()),
            },
            _ = tick.tick() => {
                let sampled = sample(context, principal, connection, &stop, families_supported);
                let update = sampled.await?;
                let bytes = encode_feed_update(&update)?;
                if last.as_ref() != Some(&bytes) {
                    let mut writer = super::host_metadata::AuthorizedWriter {
                        writer: &mut *send,
                        context,
                        principal,
                        mode: SessionMode::View,
                        stop: Some(&stop),
                    };
                    let written = write_frame(&mut writer, FEED_UPDATE, &bytes);
                    super::host_metadata::until(connection, Some(stop.clone()), written).await?;
                    last = Some(bytes);
                }
            }
        }
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
