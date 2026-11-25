//! Wire shapes of the screen strand's IPC (`ipc_protocol_screen.rs`).
use super::*;
use crate::screen::core::wire_binary::{
    FrameRecord, GopHeader, GOP_HEADER_BYTES, RECORD_HEADER_BYTES,
};

#[test]
fn batches_task_b6_session_budget_private_relay_keeps_v1_and_rejects_oversize_before_read() {
    use crate::screen::core::{frame_budget::legacy_limits, frame_size_caps::PacketBudget};
    let local = legacy_limits(24 * 1024 * 1024, None).unwrap();
    let hello = serde_json::from_value(
        serde_json::json!({"authorization":{"view":"allow","control":"allow"},
        "platform":"windows","approval":"not_required"}),
    )
    .unwrap();
    let accepted = RelayHello::Accepted {
        hello,
        path: "direct".into(),
        local_limits: Some(local),
        rate_ceiling: RateCeilingState::Automatic,
        set_decoders: false,
    };
    let bytes = serde_json::to_vec(&accepted).unwrap();
    let accepted: RelayHello = serde_json::from_slice(&bytes).unwrap();
    let RelayHello::Accepted {
        hello,
        local_limits,
        ..
    } = accepted
    else {
        panic!("not Accepted")
    };
    assert!(hello.frame_contract.is_none());
    assert_eq!(local_limits, Some(local));
    let packet = PacketBudget::for_receive(hello.frame_contract, local_limits).unwrap();
    let bound = packet.record_bytes + GOP_HEADER_BYTES + RECORD_HEADER_BYTES;
    let mut bytes = vec![RELAY_RECORD];
    bytes.extend_from_slice(&((bound + 1) as u32).to_be_bytes());
    let error = read_relay_budgeted(&mut bytes.as_slice(), usize::MAX, bound).unwrap_err();
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidData,
        "no payload was supplied"
    );
}

/// 4.10 (BK6): „Codec“ and „Dekodierer“ travel in `StreamRequest` and
/// narrow the offers; the relay says what became of the data rate.
#[test]
fn bildkette_task_codec_stream_request_and_ceiling_state() {
    use crate::screen::core::negotiate::{CodecKind, H264Profile};
    // A request of v0.5.196 without `codec`/`decoder`: automatic, and
    // automatic writes no field.
    let old = serde_json::json!({"mode":"view","decoders":[],"max_fps":60});
    let mut request: StreamRequest = serde_json::from_value(old).unwrap();
    assert_eq!((request.codec, request.decoder), (None, None));
    let back = serde_json::to_value(&request).unwrap();
    assert!(back.get("codec").is_none() && back.get("decoder").is_none());
    request.decoders = vec![
        CodecOffer::video(CodecKind::Av1, false, true),
        CodecOffer::video(CodecKind::Av1, false, false),
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::tiles(),
    ];
    assert_eq!(request.chosen_decoders(), request.decoders);
    request.codec = Some(CodecSetting::Av1);
    request.decoder = Some(DecoderPreference::Software);
    let chosen = request.chosen_decoders();
    // A chosen codec offers no tiles (plan 2026-11-18 „Netzrate“).
    assert_eq!(
        chosen,
        vec![CodecOffer::video(CodecKind::Av1, false, false)]
    );
    // „Verlustfrei“ offers the tiles alone (fix 2026-11-19).
    request.codec = Some(CodecSetting::Lossless);
    request.decoder = None;
    assert_eq!(request.chosen_decoders(), vec![CodecOffer::tiles()]);
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["codec"], "lossless");
    assert_eq!(
        serde_json::from_value::<StreamRequest>(json).unwrap(),
        request
    );
    // What became of „Höchste Datenrate“: said, not silent.
    assert_eq!(
        RateCeilingState::of(None, false),
        RateCeilingState::Automatic
    );
    assert_eq!(
        RateCeilingState::of(Some(8_000_000), true),
        RateCeilingState::Applied
    );
    assert_eq!(
        RateCeilingState::of(Some(8_000_000), false),
        RateCeilingState::UnsupportedByHost
    );
    let hello = serde_json::json!({"kind":"accepted","hello":{"authorization":
        {"view":"allow","control":"allow"},"platform":"linux","approval":"not_required"},
        "path":"direct"});
    let RelayHello::Accepted { rate_ceiling, .. } = serde_json::from_value(hello).unwrap() else {
        panic!("not Accepted");
    };
    assert_eq!(rate_ceiling, RateCeilingState::Automatic);
}

/// B-09: „Höchste Datenrate“ travels in `StreamRequest`; a request of
/// v0.5.196 without it means automatic, and automatic adds no field.
#[test]
fn bildkette_task_rate_stream_request_carries_the_max_rate() {
    let old = serde_json::json!({"mode":"view","decoders":[],"max_fps":60});
    let request: StreamRequest = serde_json::from_value(old).unwrap();
    assert_eq!(request.max_rate_bps, None);
    let back = serde_json::to_value(&request).unwrap();
    assert!(back.get("max_rate_bps").is_none());
    let limited = StreamRequest {
        max_rate_bps: Some(20_000_000),
        ..request
    };
    let json = serde_json::to_value(&limited).unwrap();
    assert_eq!(json["max_rate_bps"], 20_000_000);
    assert_eq!(
        serde_json::from_value::<StreamRequest>(json).unwrap(),
        limited
    );
}

#[test]
fn batches_task_b6_extended_4k_ipc_contract_is_complete() {
    use crate::screen::core::frame_budget::{FrameCapabilities, FrameContract};
    let caps =
        FrameCapabilities::admitted(FrameSize::new(16384, 16384), 768 * 1024 * 1024).unwrap();
    let contract = FrameContract::negotiate(Some(caps), Some(caps)).unwrap();
    let header = GopHeader {
        gop_id: 1,
        frame_contract: contract,
        family_layout_revision: None,
        kind: crate::screen::core::wire_binary::StreamKind::Video,
        codec: crate::screen::core::negotiate::CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(3840, 2160),
    };
    let record = FrameRecord {
        frame_id: 1,
        capture_ts_us: 100,
        flags: crate::screen::core::wire_binary::FLAG_KEYFRAME,
        payload: vec![0],
    };
    let bytes = encode_relay_record(&header, &record).unwrap();
    assert_eq!(decode_relay_record(&bytes).unwrap(), (header, record));
}

#[test]
fn screen_task_ipc_screen_wire_shapes() {
    let target = PeerOpenTarget::Direct {
        contact_id: "contact".into(),
    };
    let request = ScreenIpcRequest::Bench {
        target,
        options: BenchRequest {
            codec: "tiles".into(),
            size: FrameSize::new(1280, 720),
            fps: 30,
            seconds: 10,
            source: None,
            delivery: None,
        },
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["kind"], "bench");
    assert_eq!(json["options"]["size"]["width"], 1280);
    let back: ScreenIpcRequest = serde_json::from_value(json).unwrap();
    assert_eq!(back, request);
    let stop: ScreenIpcRequest = serde_json::from_str(r#"{"kind":"host_stop"}"#).unwrap();
    assert_eq!(stop, ScreenIpcRequest::HostStop { session: None });
    let reply = ScreenIpcReply::Stopped { count: 2 };
    assert_eq!(
        serde_json::to_string(&reply).unwrap(),
        r#"{"kind":"stopped","count":2}"#
    );

    // Relay frames keep their length limit before allocating.
    let mut wire = Vec::new();
    write_relay(&mut wire, RELAY_CLOCK, &(-5i64).to_be_bytes()).unwrap();
    let (tag, payload) = read_relay(&mut wire.as_slice(), 8).unwrap();
    assert_eq!((tag, payload.len()), (RELAY_CLOCK, 8));
    let too_long = read_relay(&mut wire.as_slice(), 7).unwrap_err();
    assert_eq!(too_long.kind(), std::io::ErrorKind::InvalidData);

    // A picture record crosses the relay unchanged; a length that does
    // not match is refused.
    let header = GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id: 7,
        kind: crate::screen::core::wire_binary::StreamKind::Video,
        codec: crate::screen::core::negotiate::CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(640, 360),
    };
    let record = FrameRecord {
        frame_id: 42,
        capture_ts_us: 1_000,
        flags: crate::screen::core::wire_binary::FLAG_KEYFRAME,
        payload: vec![1, 2, 3],
    };
    let body = encode_relay_record(&header, &record).unwrap();
    assert_eq!(
        decode_relay_record(&body).unwrap(),
        (header, record.clone())
    );
    let cut = &body[..body.len() - 1];
    assert!(decode_relay_record(cut).is_err());

    // A family layout belongs to the actual v2 picture header across IPC.
    // Legacy v2 extensions still have no layout revision.
    // Versioned limits stay within the 24-image budget (d577bab8).
    use crate::screen::core::frame_budget::{FrameCapabilities, FrameContract, FRAME_PROTOCOL};
    let limits = FrameCapabilities::legacy_resources(FrameSize::new(4096, 4096)).frames;
    let mut header = header;
    header.frame_contract = Some(FrameContract {
        version: FRAME_PROTOCOL,
        limits,
        hybrid_regions: true,
    });
    let body = encode_relay_record(&header, &record).unwrap();
    assert_eq!(
        decode_relay_record(&body).unwrap(),
        (header, record.clone())
    );
    header.family_layout_revision = Some(u64::MAX);
    let body = encode_relay_record(&header, &record).unwrap();
    assert_eq!(decode_relay_record(&body).unwrap(), (header, record));
    header.family_layout_revision = Some(0);
    assert!(encode_relay_record(
        &header,
        &FrameRecord {
            frame_id: 43,
            capture_ts_us: 1_001,
            flags: 1,
            payload: vec![1],
        }
    )
    .is_err());
}

/// User 2026-11-25: „Übertragung“ travels in `StreamRequest`; a request
/// before it means the default, and the default writes no field.
#[test]
fn delivery_task_stream_request_carries_the_delivery() {
    use crate::screen::core::delivery::Delivery;
    let old = serde_json::json!({"mode":"view","decoders":[],"max_fps":60});
    let request: StreamRequest = serde_json::from_value(old).unwrap();
    assert_eq!(request.delivery, None);
    assert!(serde_json::to_value(&request)
        .unwrap()
        .get("delivery")
        .is_none());
    let reliable = StreamRequest {
        delivery: Some(Delivery::Reliable),
        ..request
    };
    let json = serde_json::to_value(&reliable).unwrap();
    assert_eq!(json["delivery"], "reliable");
    assert_eq!(
        serde_json::from_value::<StreamRequest>(json).unwrap(),
        reliable
    );
}
