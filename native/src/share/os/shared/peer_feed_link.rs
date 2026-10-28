//! One feed session of `peer_feed` (design 5.2): the probe handshake that
//! learns the rights without asking the host's user, the subscription on
//! the metadata channel and the update loop.
use super::{Context, PeerFeedView};
use crate::screen::core::family_protocol::FAMILY_PROTOCOL;
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
use crate::share::node_screen::ScreenLink;
use std::io;
use std::time::Duration;
use tokio::sync::watch;

/// Per handshake step on the active route (design 7: kept).
const HANDSHAKE: Duration = Duration::from_secs(20);
const OLDER_VERSION: &str = "Gerät hat eine ältere Version";

/// Why a feed session ended and whether the connection was lost (a lost
/// connection restarts on the next network event; a refusal waits for a
/// presence event or a new profile revision).
pub(super) struct Ended {
    pub(super) reason: String,
    pub(super) connection_lost: bool,
}
impl Ended {
    fn refused(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            connection_lost: false,
        }
    }
    fn lost(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            connection_lost: true,
        }
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
) -> Result<(ScreenHelloOk, iroh::endpoint::Connection, Streams), Ended> {
    let connection = link.connection.clone();
    let (mut send, mut recv) =
        crate::share::route_protocol::run_with_route(&connection, HANDSHAKE, None, async {
            connection.accept_bi().await.map_err(io::Error::other)
        })
        .await
        .map_err(|error| Ended::lost(error.to_string()))?;
    let server = crate::share::route_protocol::run_with_route(
        &connection,
        HANDSHAKE,
        None,
        recv_server_hello(&mut recv),
    )
    .await
    .map_err(|error| Ended::lost(error.to_string()))?;
    if !wire::supports_version(server.protocol_version) {
        return Err(Ended::refused(OLDER_VERSION));
    }
    if mode != SessionMode::Probe && server.family_feed_version != Some(FAMILY_FEED_VERSION) {
        return Err(Ended::refused(OLDER_VERSION));
    }
    let mut hello = link
        .client_hello(&server, &viewer_request(mode))
        .map_err(|error| Ended::refused(error.to_string()))?;
    // Outside the signed transcript, as in `viewer_client::handshake_for`.
    hello.family_version = server.family_version.filter(|v| *v == FAMILY_PROTOCOL);
    hello.icon_version = None;
    hello.metadata_only = mode != SessionMode::Probe && hello.family_version.is_some();
    hello.frame_capabilities = None;
    connection.set_max_concurrent_uni_streams(iroh::endpoint::VarInt::from_u32(0));
    send_client_hello(&mut send, &hello)
        .await
        .map_err(|error| Ended::lost(error.to_string()))?;
    let reply = crate::share::route_protocol::run_with_route(
        &connection,
        HANDSHAKE,
        None,
        recv_hello_reply(&mut recv),
    )
    .await
    .map_err(|error| Ended::lost(error.to_string()))?;
    match reply {
        HelloReply::Accepted(accepted) => Ok((accepted, connection, (send, recv))),
        HelloReply::Refused(refused) => Err(Ended::refused(refusal_text(&refused.code))),
        HelloReply::Pending(_) => {
            // Never a question from the background: withdraw at once.
            connection.close(iroh::endpoint::VarInt::from_u32(0), b"feed does not ask");
            Err(Ended::refused(ASKS))
        }
    }
}

type Streams = (iroh::endpoint::SendStream, iroh::endpoint::RecvStream);
const ASKS: &str =
    "„Bildschirm ansehen“ steht auf diesem Gerät auf „Nachfragen“; Fenster und Dienste erscheinen mit „Erlauben“";

/// A new screen connection to the device (blocking).
fn link(context: &Context) -> Result<ScreenLink, Ended> {
    context
        .service
        .screen_link(&context.target)
        .map_err(|error| Ended::lost(error.detail()))
}

/// Probe, subscribe, apply every update; the reason the feed ended, `None`
/// when `observe` stopped it.
pub(super) fn feed(context: &Context, mut stopped: watch::Receiver<bool>) -> Option<Ended> {
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
        CapabilityDecision::Ask => return Some(Ended::refused(ASKS)),
        _ => return Some(Ended::refused(refusal_text("permission_denied"))),
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

async fn subscribed(context: &Context, link: ScreenLink) -> Ended {
    let (_, connection, (mut send, mut recv)) = match handshake(link, SessionMode::View).await {
        Ok(opened) => opened,
        Err(error) => return error,
    };
    if let Err(error) = send_subscribe_feed(&mut send).await {
        return Ended::lost(format!("Gerätefeed nicht abonniert: {error}"));
    }
    context.show(|view| {
        view.live = true;
        view.unavailable = None;
    });
    // No receive limit: QUIC's idle detection (20 s) ends a dead feed.
    let ended = loop {
        let (tag, payload) = match wire::read_frame(&mut recv, MAX_WINDOWS_BYTES).await {
            Ok(frame) => frame,
            Err(error) => break Ended::lost(format!("Gerätefeed beendet: {error}")),
        };
        if tag == wire::ERROR {
            let error: Result<ScreenWireError, _> = wire::from_json(&payload);
            let reason = error.map_or_else(|e| e.to_string(), |error| refusal_text(&error.code));
            break Ended::refused(reason);
        }
        if tag != FEED_UPDATE {
            continue;
        }
        let update = match decode_feed_update(&payload) {
            Ok(update) => update,
            Err(error) => break Ended::refused(format!("Ungültiges Gerätefeed-Update: {error}")),
        };
        let services = update.services.clone().unwrap_or_default();
        context.show(|view: &mut PeerFeedView| {
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
