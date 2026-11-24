//! Tests of plan 2026-11-14 „Bildkette“ B-35 and B-36 (prefix
//! `bildkette_task_recover_`): the stale time from round trip and frame
//! interval, the capability `gop_continue_v1` with old peers in both roles,
//! the continuation only from a confirmed, decodable frame, the fallback to
//! a key frame, and the viewer that keeps its decoder over a continuation.
//! Not registered yet (`docs/tests/register.tsv`): the manager adds the rows.
use std::time::Duration;

use iroh::endpoint::Connection;

use super::{negotiated, DecoderContinuity, ResetFacts, StreamStart, GOP_CONTINUE_V1};
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::gop_stream::{
    accept_picture_streams, picture_channel, GopWriter, PictureEvent,
};
use crate::screen::core::host_backend::{ScreenEncoder, TilesScreenEncoder};
use crate::screen::core::negotiate::{CodecKind, H264Profile};
use crate::screen::core::pacing::{
    in_flight_limit, stale_after_us, PaceDecision, Pacer, SentFrame, MAX_IN_FLIGHT_FRAMES,
    STALE_STREAM_US,
};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::PathKind;
use crate::screen::core::ref_recovery::{ConfirmedReference, LtrSlots, Recovery, RefHistory};
use crate::screen::core::wire_binary::{
    FrameRecord, GopHeader, RecordSequence, StreamKind, FLAG_CONTINUE, FLAG_KEYFRAME,
};
use crate::screen::core::wire_control::ViewerFrame;
use crate::screen::core::wire_hello::{ScreenClientHello, SessionMode};

const TARGET: u64 = 6_000_000;
/// One frame interval at 60 frames/s.
const AT_60: u64 = 16_666;

fn sent(gop_id: u64, frame_id: u64, bytes: u64, sent_us: u64, keyframe: bool) -> SentFrame {
    SentFrame {
        gop_id,
        frame_id,
        bytes,
        sent_us,
        keyframe,
    }
}

fn record(frame_id: u64, flags: u8) -> FrameRecord {
    FrameRecord {
        frame_id,
        capture_ts_us: frame_id,
        flags,
        payload: vec![0; 4],
    }
}

fn header(gop_id: u64, codec: CodecKind) -> GopHeader {
    GopHeader {
        gop_id,
        frame_contract: None,
        family_layout_revision: None,
        kind: StreamKind::Video,
        codec,
        profile: (codec == CodecKind::H264).then_some(H264Profile::High),
        size: FrameSize::new(64, 64),
    }
}

/// Every condition of a continuation holds.
fn continuable(last_good: Option<u64>) -> ResetFacts {
    ResetFacts {
        negotiated: true,
        video: true,
        keyframe_due: false,
        encoder_running: true,
        stage_continues: true,
        last_good,
    }
}

/// B-35: max(4 × smoothed round trip, 3 frame intervals, one second) plus
/// the time the line needs for the unacknowledged bytes; the frame limit
/// follows it (the second since 2026-11-24: a reliable stream whose viewer
/// decoded slowly for a moment is not stale).
#[test]
fn bildkette_task_recover_stale_time_from_rtt_and_frame_interval() {
    // LAN, 1 ms round trip at 60 frames/s: the floor of one second decides.
    assert_eq!(stale_after_us(1_000, AT_60, 0, TARGET), STALE_STREAM_US);
    // WAN, 80 ms round trip: still the floor.
    assert_eq!(stale_after_us(80_000, AT_60, 0, TARGET), STALE_STREAM_US);
    // A satellite link of 400 ms: four round trips decide.
    assert_eq!(stale_after_us(400_000, AT_60, 0, TARGET), 1_600_000);
    // A 300 KB key frame at 6 Mbit/s adds the 400 ms the line needs.
    assert_eq!(
        stale_after_us(5_000, AT_60, 300_000, TARGET),
        STALE_STREAM_US + 400_000
    );
    // Without a known round trip the floor remains.
    assert_eq!(stale_after_us(0, 33_333, 0, TARGET), STALE_STREAM_US);
    // Twice the frames of the stale time, at most the memory bound.
    assert_eq!(in_flight_limit(3 * AT_60, AT_60), 8);
    assert_eq!(in_flight_limit(10_000_000, 4_166), MAX_IN_FLIGHT_FRAMES);

    // The pacer resets right after the stale time of its timing: 750 bytes
    // in flight need 1 ms at 6 Mbit/s.
    let direct = PathKind::Direct;
    let mut pacer = Pacer::default();
    pacer.set_timing(1_000, AT_60);
    pacer.on_frame_sent(sent(1, 1, 750, 0, true));
    let stale = STALE_STREAM_US + 1_000;
    assert_eq!(
        pacer.stale_limits(TARGET),
        (stale, in_flight_limit(stale, AT_60))
    );
    assert_eq!(
        pacer.decide(stale, TARGET, direct, 1_000),
        PaceDecision::Encode { keyframe: false }
    );
    assert_eq!(
        pacer.decide(stale + 1, TARGET, direct, 1_000),
        PaceDecision::ResetStream
    );
    // A continued reset forgets the frames but asks for no key frame.
    pacer.on_stream_reset_continued();
    assert_eq!((pacer.frames_in_flight(), pacer.unacked_bytes()), (0, 0));
    assert_eq!(pacer.resets(), 1);
    assert_eq!(
        pacer.decide(stale + 2, TARGET, direct, 1_000),
        PaceDecision::Encode { keyframe: false }
    );
    // A pacer without timing keeps the old second.
    let mut old = Pacer::default();
    old.on_frame_sent(sent(1, 1, 750, 0, true));
    assert_ne!(
        old.decide(STALE_STREAM_US, TARGET, direct, 1_000),
        PaceDecision::ResetStream
    );
    assert_eq!(
        old.decide(STALE_STREAM_US + 1, TARGET, direct, 1_000),
        PaceDecision::ResetStream
    );
}

fn old_client_hello() -> serde_json::Value {
    serde_json::json!({"protocol_version":1,"relation_kind":"direct",
        "relation_id":"fixture","device_id":"viewer","device_name":"viewer","public_key":"key",
        "fingerprint":"pin","node_id":"node","client_nonce":"nonce","proof":"proof",
        "mode":"view","source":"synthetic",
        "decoders":[{"codec":"h264","profile":"high","hardware":true},{"codec":"tiles"}],
        "display_size":{"width":1920,"height":1080},"max_fps":60})
}

/// `gop_continue_v1` with a peer of v0.5.196 in both roles.
#[test]
fn bildkette_task_recover_hello_with_old_peers() {
    // Old viewer, new host: no field, no continuation; the host resets as
    // before even with a confirmed frame.
    let old: ScreenClientHello = serde_json::from_value(old_client_hello()).unwrap();
    assert_eq!(old.gop_continue, None);
    assert!(!negotiated(&old));
    let towards_old = ResetFacts {
        negotiated: negotiated(&old),
        ..continuable(Some(9))
    };
    assert_eq!(towards_old.continue_from(), None);
    // Without the capability the new hello is the old one byte for byte.
    let unchanged = serde_json::to_value(&old).unwrap();
    assert!(unchanged.get("gop_continue").is_none());

    // New viewer, old host: the old host reads the hello with the fields it
    // knows (serde ignores the rest, no `deny_unknown_fields`) and resets as
    // before; it never sends `FLAG_CONTINUE`.
    let mut new = old.clone();
    new.gop_continue = Some(GOP_CONTINUE_V1);
    let bytes = serde_json::to_vec(&new).unwrap();
    #[derive(serde::Deserialize)]
    struct OldHostView {
        protocol_version: u16,
        mode: SessionMode,
        max_fps: u16,
    }
    let read: OldHostView = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (read.protocol_version, read.mode, read.max_fps),
        (1, SessionMode::View, 60)
    );
    // New peers: the host may continue.
    let both: ScreenClientHello = serde_json::from_slice(&bytes).unwrap();
    assert!(negotiated(&both));
    assert_eq!(
        ResetFacts {
            negotiated: negotiated(&both),
            ..continuable(Some(9))
        }
        .continue_from(),
        Some(9)
    );
    // A later version is not this one.
    new.gop_continue = Some(GOP_CONTINUE_V1 + 1);
    assert!(!negotiated(&new));
}

/// Only a frame the viewer acknowledged on the stream the host writes is a
/// reference: `FRAME_ACK` means „arrived“ (the reader sends it before the
/// decoder sees the frame), and on the ordered stream that began with a key
/// frame or a continuation every earlier frame arrived as well.
#[test]
fn bildkette_task_recover_continues_only_from_a_confirmed_decodable_frame() {
    let mut confirmed = ConfirmedReference::default();
    assert_eq!(confirmed.last_good(), None);
    confirmed.opened(1, false);
    confirmed.acked(1, 3);
    confirmed.acked(1, 2);
    assert_eq!(
        confirmed.last_good(),
        Some(3),
        "an older ack never lowers it"
    );
    // Another stream than the one written does not count.
    confirmed.acked(7, 9);
    assert_eq!(confirmed.last_good(), Some(3));
    // After the reset a late ack of the old stream names a frame the
    // encoder may already have invalidated.
    confirmed.stream_reset();
    confirmed.acked(1, 5);
    assert_eq!(confirmed.last_good(), Some(3));
    assert_eq!(continuable(confirmed.last_good()).continue_from(), Some(3));
    // The continuation keeps the reference until its own frames arrive.
    confirmed.opened(2, true);
    assert_eq!(confirmed.last_good(), Some(3));
    confirmed.acked(2, 6);
    assert_eq!(confirmed.last_good(), Some(6));
    // A key frame stream begins a new chain; a new encoder (also after a
    // decode error, which the viewer answers with a key frame request)
    // forgets everything.
    confirmed.opened(3, false);
    assert_eq!(confirmed.last_good(), None);
    assert_eq!(continuable(confirmed.last_good()).continue_from(), None);
    confirmed.acked(3, 8);
    assert_eq!(confirmed.last_good(), Some(8));
    confirmed.restart();
    assert_eq!(confirmed.last_good(), None);
}

/// Without reference control, without the frame in the encoder, without a
/// stage that passes the wish on or with a key frame due: a key frame.
#[test]
fn bildkette_task_recover_falls_back_to_a_key_frame() {
    // An encoder without reference control (the tiles here; OpenH264, the
    // software MFT, VA-API and MediaCodec keep the trait's default).
    let mut tiles = TilesScreenEncoder::new(FrameSize::new(64, 64)).unwrap();
    assert_eq!(tiles.recover_references(5), Recovery::KeyframeNeeded);
    // NVENC: the confirmed frame has left the DPB of five.
    let mut history = RefHistory::new(5);
    for frame in 1..=5 {
        history.encoded(frame, frame == 1);
    }
    assert_eq!(history.invalidate_after(3), Some(vec![4, 5]));
    for frame in 6..=10 {
        history.encoded(frame, false);
    }
    assert_eq!(history.invalidate_after(3), None);
    // AMF: no long-term reference is old enough.
    let mut ltr = LtrSlots::new(60);
    ltr.plan(1, true);
    assert!(!ltr.recover(0));
    // The host keeps the old reset for every missing condition.
    let all = continuable(Some(4));
    assert_eq!(all.continue_from(), Some(4));
    for facts in [
        ResetFacts {
            stage_continues: false,
            ..all
        },
        ResetFacts {
            encoder_running: false,
            ..all
        },
        ResetFacts {
            keyframe_due: true,
            ..all
        },
        ResetFacts {
            video: false,
            ..all
        },
        ResetFacts {
            last_good: None,
            ..all
        },
    ] {
        assert_eq!(facts.continue_from(), None, "{facts:?}");
    }
    // The viewer whose reference is missing after all asks for a key frame
    // and decodes nothing of the stream (the last good picture stays).
    let mut chain = DecoderContinuity::default();
    assert_eq!(
        chain.start(&record(4, FLAG_CONTINUE)),
        StreamStart::RequestKeyframe
    );
    assert_eq!(
        chain.start(&record(5, FLAG_KEYFRAME)),
        StreamStart::OpenDecoder
    );
    chain.broken();
    assert_eq!(
        chain.start(&record(9, FLAG_CONTINUE)),
        StreamStart::RequestKeyframe
    );
    assert!(!chain.intact());
}

/// The viewer keeps its decoder over a continuation stream, and the wire
/// carries the continuation only where it is allowed.
#[test]
fn bildkette_task_recover_viewer_keeps_the_decoder_over_a_continuation() {
    let mut chain = DecoderContinuity::default();
    assert_eq!(
        chain.start(&record(1, FLAG_KEYFRAME)),
        StreamStart::OpenDecoder
    );
    // The stream broke off (`Broken`): its decoder stays and continues.
    assert_eq!(
        chain.start(&record(4, FLAG_CONTINUE)),
        StreamStart::KeepDecoder
    );
    assert_eq!(
        chain.start(&record(8, FLAG_CONTINUE)),
        StreamStart::KeepDecoder
    );
    assert!(chain.intact());
    // A stream that starts with neither is refused.
    assert_eq!(chain.start(&record(9, 0)), StreamStart::RequestKeyframe);

    let h264 = header(2, CodecKind::H264);
    let mut sequence = RecordSequence::default();
    sequence.accept(&h264, &record(7, FLAG_CONTINUE)).unwrap();
    sequence.accept(&h264, &record(8, 0)).unwrap();
    let first = |flags| RecordSequence::default().accept(&h264, &record(7, flags));
    assert!(first(0).is_err(), "neither key frame nor continuation");
    assert!(first(FLAG_CONTINUE | FLAG_KEYFRAME).is_err());
    let mut later = RecordSequence::default();
    later.accept(&h264, &record(1, FLAG_KEYFRAME)).unwrap();
    assert!(later.accept(&h264, &record(2, FLAG_CONTINUE)).is_err());
    let tiles = header(2, CodecKind::Tiles);
    assert!(RecordSequence::default()
        .accept(&tiles, &record(1, FLAG_CONTINUE))
        .is_err());
}

const ALPN: &[u8] = b"bildkette-recover/1";

async fn endpoint() -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_address_lookup()
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .expect("loopback bind address")
        .alpns(vec![ALPN.to_vec()])
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
    let (outgoing, incoming) = tokio::join!(viewer.connect(address, ALPN), async {
        host.accept().await.expect("incoming connection").await
    });
    (outgoing.expect("viewer side"), incoming.expect("host side"))
}

/// Over a real QUIC connection: the decoder is slow (nothing is taken from
/// the picture queue), the host acknowledged frames 1 to 5 of stream 1 and
/// continues on stream 2; the continuation reaches the decoder only after
/// every record of stream 1 and its end, with `FLAG_CONTINUE` and no key
/// frame flag.
#[tokio::test]
async fn bildkette_task_recover_continuation_reaches_the_decoder_after_the_acknowledged_frames() {
    let (viewer, host) = (endpoint().await, endpoint().await);
    let (viewer_side, host_side) = connect(&viewer, &host).await;
    let (events, mut pictures) = picture_channel();
    let (acks, mut acked) = tokio::sync::mpsc::channel(64);
    let reader = tokio::spawn(accept_picture_streams(viewer_side.clone(), events, acks));
    let grant = crate::screen::core::capture_test_support::grant(FrameSize::new(64, 64), None);
    let frame = |id: u64| {
        CapturedRecords::from_allocator(&grant, 64, 1, || Ok(vec![vec![id as u8; 16]])).unwrap()
    };
    let mut first = GopWriter::open(&host_side, header(1, CodecKind::H264))
        .await
        .expect("stream 1");
    for id in 1..=6 {
        first
            .write_frame_admitted(id, id, id == 1, frame(id))
            .await
            .expect("frame of stream 1");
    }
    // The queue holds four records; the reader acknowledged the fifth and
    // waits with it, the sixth stays unread.
    loop {
        match tokio::time::timeout(Duration::from_secs(5), acked.recv()).await {
            Ok(Some(ViewerFrame::FrameAck(ack))) if ack.frame_id == 5 => break,
            Ok(Some(_)) => {}
            other => panic!("no acknowledgement of frame 5: {other:?}"),
        }
    }
    let mut second = GopWriter::open(&host_side, header(2, CodecKind::H264))
        .await
        .expect("stream 2");
    second.mark_continuation();
    second
        .write_frame_admitted(7, 7, false, frame(7))
        .await
        .expect("continuation");
    tokio::time::sleep(Duration::from_millis(100)).await;
    first.reset();
    let mut order = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), pictures.recv())
            .await
            .expect("picture event in time")
            .expect("picture channel open");
        match event {
            PictureEvent::Record { header, record, .. } => {
                order.push((header.gop_id, record.frame_id, record.flags));
                if record.frame_id == 7 {
                    break;
                }
            }
            PictureEvent::Broken { gop_id } => order.push((gop_id, 0, 0)),
        }
    }
    let (continuation, before) = order.split_last().expect("events");
    assert_eq!(*continuation, (2, 7, FLAG_CONTINUE), "{order:?}");
    assert!(
        before.contains(&(1, 0, 0)),
        "stream 1 ended first: {order:?}"
    );
    for id in 1..=5 {
        let flags = if id == 1 { FLAG_KEYFRAME } else { 0 };
        assert!(before.contains(&(1, id, flags)), "frame {id}: {order:?}");
    }
    reader.abort();
}
