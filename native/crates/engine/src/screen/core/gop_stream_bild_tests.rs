//! Tests of the picture transport rules of plan 2026-11-14 P4 (prefix
//! `verbindungen_task_bild_`): presence only for `view` and `control` (E3)
//! and the host's control stream above the picture groups (E4).
use std::time::Duration;

use iroh::endpoint::Connection;

use super::{
    picture_session, prioritize_control, GopWriter, PicturePresence, CONTROL_STREAM_PRIORITY,
};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::{GopHeader, StreamKind};
use crate::screen::core::wire_hello::SessionMode;

/// ALPN of the loopback screen connection.
const CONTROL: &[u8] = b"verbindungen-bild-control/1";

async fn endpoint() -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_address_lookup()
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .expect("loopback bind address")
        .alpns(vec![CONTROL.to_vec()])
        .bind()
        .await
        .expect("bind loopback endpoint")
}

/// A connection from `viewer` to `host`: (viewer side, host side).
async fn connect(viewer: &iroh::Endpoint, host: &iroh::Endpoint) -> (Connection, Connection) {
    let address = iroh::EndpointAddr::from_parts(
        host.id(),
        host.bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );
    let (outgoing, incoming) = tokio::join!(viewer.connect(address, CONTROL), async {
        host.accept().await.expect("incoming connection").await
    });
    (outgoing.expect("viewer side"), incoming.expect("host side"))
}

#[test]
fn verbindungen_task_bild_presence_only_for_view_and_control() {
    assert!(picture_session(SessionMode::View, false));
    assert!(picture_session(SessionMode::Control, false));
    assert!(!picture_session(SessionMode::Probe, false));
    assert!(!picture_session(SessionMode::Preview, false));
    // A metadata channel (device feed, lists, icons) runs as `view`.
    assert!(!picture_session(SessionMode::View, true));
    let guard: crate::share::node_screen::InteractiveGuard =
        crate::share::node_screen::enter_interactive(
            crate::share::node_screen::InteractiveKind::ScreenViewer,
        );
    assert_eq!(
        guard.kind(),
        crate::share::node_screen::InteractiveKind::ScreenViewer
    );
}

/// The group view raises a tile's preview session to the full view with
/// `SET_MODE`, without a new hello: the presence follows what the session
/// streams, on the viewer's and on the host's side.
#[test]
fn verbindungen_task_bild_presence_follows_a_preview_raised_to_the_full_view() {
    let mut tile = PicturePresence::viewer(SessionMode::Preview, false);
    assert!(!tile.active(), "a preview does not count");
    tile.set_full(true);
    assert!(tile.active(), "raised to the full view it counts");
    tile.set_full(true);
    assert!(tile.active());
    tile.set_full(false);
    assert!(!tile.active(), "back as a tile it does not");
    // A full view counts from its start and stops counting as a tile.
    let mut view = PicturePresence::viewer(SessionMode::Control, false);
    assert!(view.active());
    view.set_full(false);
    assert!(!view.active());
    // A probe and a metadata channel never stream pictures.
    let mut probe = PicturePresence::viewer(SessionMode::Probe, false);
    probe.set_full(true);
    assert!(!probe.active());
    let mut metadata = PicturePresence::viewer(SessionMode::View, true);
    metadata.set_full(true);
    assert!(!metadata.active());
    // The host's pipeline: a preview pipeline counts once it streams the full view.
    let mut pipeline = PicturePresence::host(false);
    assert!(!pipeline.active());
    pipeline.set_full(true);
    assert!(pipeline.active());
    assert!(PicturePresence::host(true).active());
}

#[tokio::test]
async fn verbindungen_task_bild_control_stream_above_picture_groups() {
    let (viewer, host) = (endpoint().await, endpoint().await);
    let (_viewer_control, host_control) = connect(&viewer, &host).await;
    let (host_send, _host_recv) = host_control.open_bi().await.expect("control stream");
    prioritize_control(&host_send);
    let control = host_send.priority().expect("open control stream");
    assert_eq!(control, CONTROL_STREAM_PRIORITY);
    // A picture group opens its stream as the host's pipeline does.
    let header = GopHeader {
        gop_id: 1,
        frame_contract: None,
        family_layout_revision: None,
        kind: StreamKind::Video,
        codec: CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(64, 64),
    };
    let opening = GopWriter::open(&host_control, header);
    let group = tokio::time::timeout(Duration::from_secs(10), opening)
        .await
        .expect("the picture group opens in time")
        .expect("picture group stream");
    let pictures = group.send.priority().expect("open picture stream");
    assert!(
        control > pictures,
        "the control stream ({control}) goes before the picture groups ({pictures})"
    );
}
