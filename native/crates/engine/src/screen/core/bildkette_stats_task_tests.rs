//! BK0 (plan 2026-11-14 „Bildkette“): the chain facts and station times of
//! a host session, their way into `STATS` and `se screen sessions`, and
//! both directions of an older peer (v0.5.196).
use serde::Deserialize;

use super::*;
use crate::screen::core::frame_timing::{
    CaptureFacts, CaptureFallback, CapturePath, FrameTimingRequest, SkewedUs, FRAME_TIMING,
    FRAME_TIMING_VERSION,
};
use crate::screen::core::rate_control::PathKind;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_control::{
    decode_host_frame, decode_viewer_frame, encode_host_frame, encode_viewer_frame, HostFrame,
    ViewerFrame,
};
use crate::screen::core::wire_hello::SessionMode;
use crate::screen::core::worker_wire::StreamMark;

fn view(source: &str) -> SessionView {
    SessionView {
        id: "1".into(),
        device_id: "viewer".into(),
        device_name: "Laptop".into(),
        relation: "direct".into(),
        room_id: None,
        mode: SessionMode::View,
        source: source.into(),
        codec: None,
        encoder: None,
        encoder_stage: None,
        decoder_stage: None,
        profile: None,
        path: "direct".into(),
        started_at: 0,
        bitrate_kbps: 0,
        fps: 0,
        controlling: false,
        state: SessionState::Streaming,
        chain: None,
        stations: None,
        audio: None,
    }
}

fn stats(meter: &ChainMeter, rates: &RateFacts) -> HostStats {
    HostStats {
        target_kbps: 6000,
        sent_kbps: 5800,
        fps: 30,
        path: PathKind::Direct,
        keyframes: 2,
        dropped_before_encode: 1,
        stream_resets: rates.resets,
        unacked_bytes: 0,
        frame_timing: Some(FRAME_TIMING_VERSION),
        chain: Some(meter.report(rates)),
        stations: meter.stations(),
    }
}

fn rates() -> RateFacts {
    RateFacts {
        target_fps: 60,
        wanted_fps: 60,
        capacity_fps: Some(90),
        sent_fps: 60,
        target_bps: 20_000_000,
        ceiling_bps: 20_000_000,
        user_ceiling_bps: Some(20_000_000),
        back_pressure: false,
        resets: 3,
        limit: None,
        encoder_bps: None,
        honour: None,
    }
}

/// A meter with every station measured, as a host process session of the
/// test picture fills it (`host_capture.rs`, `host_frame_send.rs`,
/// `host_mode.rs`, `host_session_control.rs`, the stream mark).
fn filled() -> ChainMeter {
    let mut meter = ChainMeter::default();
    meter.request(true);
    for micros in [1_000, 2_000, 3_000] {
        meter.record_us(Station::Capture, micros);
        meter.record_us(Station::Encode, micros * 4);
        meter.record_us(Station::WorkerHandoff, micros / 10);
        meter.record_us(Station::Send, micros / 2);
        meter.record_us(Station::InputInject, micros / 5);
    }
    // Captures at 60 frames/s: 16_666 µs apart, the last one 2 ms late.
    for at in [1_000_000, 1_016_666, 1_033_332, 1_051_998] {
        meter.on_capture_ts(at, 60);
    }
    meter.on_network(5_000_000, 2_005_000);
    meter.on_input_probe(1_000_000, 4_001_500);
    meter.set_facts(CaptureFacts {
        path: Some(CapturePath::DesktopDuplication),
        fallback: None,
        rate_capped: Some(false),
        cursor_embedded: None,
        on_gpu: Some(true),
    });
    meter.set_readbacks(7);
    meter.set_readbacks(7);
    meter.on_continued();
    meter
}

#[test]
fn bildkette_task_stats_fields_complete_with_synthetic_source() {
    let meter = filled();
    let rates = rates();
    let mut stats = stats(&meter, &rates);
    let stations = stats.stations.clone().expect("every station measured");
    for station in [
        stations.capture,
        stations.encode,
        stations.worker_handoff,
        stations.send,
        stations.tick_jitter,
        stations.input_inject,
    ] {
        assert!(station.is_some(), "{stations:?}");
    }
    assert_eq!(stations.capture.unwrap().p50_us, 2_000);
    assert_eq!(stations.encode.unwrap().p95_us, 12_000);
    // Without conversion samples (encoder-internal conversion) it is absent.
    assert_eq!(stations.convert, None);
    // Jitter: 0 and 0 µs for the even gaps, 2_000 µs for the late one.
    assert_eq!(stations.tick_jitter.unwrap().p50_us, 0);
    assert_eq!(stations.tick_jitter.unwrap().p95_us, 2_000);
    assert_eq!(
        stations.network_raw,
        Some(SkewedUs {
            p50_us: -2_995_000,
            p95_us: -2_995_000
        })
    );
    assert_eq!(stations.input_raw.map(|raw| raw.p50_us), Some(3_001_500));

    let chain = stats.chain.clone().unwrap();
    assert_eq!(chain.capture_path.as_deref(), Some("desktop_duplication"));
    assert_eq!(chain.convert.as_deref(), Some("gpu"));
    assert_eq!(chain.readbacks, Some(0));
    assert_eq!(chain.recoveries_continued, Some(1));
    assert_eq!(chain.recoveries_keyframe, Some(2));
    assert_eq!(chain.fps_limit.as_deref(), Some("viewer"));
    assert_eq!(chain.rate_limit.as_deref(), Some("ceiling"));
    assert_eq!(chain.rate_ceiling_kbps, Some(20_000));

    // The session of the test picture names it; the view keeps the stages
    // and the station times without the raw clock differences.
    let mut session = view("synthetic");
    publish(&mut session, &mut stats, true);
    let shown = session.chain.clone().unwrap();
    assert_eq!(shown.capture_path.as_deref(), Some("test_picture"));
    assert_eq!(shown.capture_fallback, None);
    let local = session.stations.clone().unwrap();
    assert!(local.network_raw.is_none() && local.input_raw.is_none());
    assert_eq!(local.capture, stations.capture);
    // Requested: the station times stay in `STATS`.
    assert!(stats.stations.is_some());

    // Every field reaches the JSON of `se screen sessions` and of `STATS`.
    let json = serde_json::to_value(&session).unwrap();
    for key in [
        "capture_path",
        "convert",
        "readbacks",
        "recoveries_continued",
        "recoveries_keyframe",
        "target_fps",
        "wanted_fps",
        "encode_capacity_fps",
        "fps_limit",
        "rate_limit",
        "rate_ceiling_kbps",
        "capture_rate_capped",
    ] {
        assert!(json["chain"].get(key).is_some(), "{key}: {json}");
    }
    for key in [
        "capture",
        "encode",
        "worker_handoff",
        "send",
        "tick_jitter",
        "input_inject",
    ] {
        assert!(json["stations"][key]["p50_us"].is_u64(), "{key}: {json}");
    }
    let (tag, payload) = encode_host_frame(&HostFrame::Stats(stats.clone())).unwrap();
    assert_eq!(
        decode_host_frame(tag, &payload).unwrap(),
        HostFrame::Stats(stats)
    );
}

#[test]
fn bildkette_task_stats_station_times_leave_only_on_request() {
    let meter = filled();
    let mut stats = stats(&meter, &rates());
    let mut session = view("monitor:0");
    publish(&mut session, &mut stats, false);
    assert!(stats.stations.is_none(), "not asked for: none on the wire");
    assert!(session.stations.is_some(), "the host's own list keeps them");
    assert_eq!(
        session.chain.unwrap().capture_path.as_deref(),
        Some("desktop_duplication")
    );

    // `FRAME_TIMING` on resets the window; off keeps the samples.
    let mut meter = filled();
    meter.request(false);
    assert!(!meter.requested() && meter.stations().is_some());
    meter.request(true);
    assert!(meter.requested() && meter.stations().is_none());
}

/// `HostStats` exactly as v0.5.196 declared it.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OldHostStats {
    target_kbps: u64,
    sent_kbps: u64,
    fps: u32,
    path: PathKind,
    keyframes: u64,
    dropped_before_encode: u64,
    stream_resets: u64,
    unacked_bytes: u64,
}

#[test]
fn bildkette_task_stats_old_peer_in_both_roles() {
    // An old viewer reads the new `STATS`: it ignores the new fields.
    let stats = stats(&filled(), &rates());
    let payload = serde_json::to_vec(&stats).unwrap();
    let old: OldHostStats = serde_json::from_slice(&payload).unwrap();
    assert_eq!(old.unacked_bytes, 0);
    assert_eq!(old.stream_resets, 3);

    // An old host sends the old shape: every new field is „not reported“,
    // and no `FRAME_TIMING` may go to it (no announcement).
    let old_json = br#"{"target_kbps":6000,"sent_kbps":5800,"fps":30,"path":"relay",
        "keyframes":2,"dropped_before_encode":1,"stream_resets":0,"unacked_bytes":4096}"#;
    let read: HostStats = serde_json::from_slice(old_json).unwrap();
    assert_eq!(read.frame_timing, None);
    assert_eq!(read.chain, None);
    assert_eq!(read.stations, None);
    // The old shape is still what a host without chain facts writes.
    let plain = HostStats {
        frame_timing: None,
        chain: None,
        stations: None,
        ..read.clone()
    };
    let json = serde_json::to_value(&plain).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 8, "{json}");
    // The old `se screen sessions` shape stays without the new keys.
    let json = serde_json::to_value(view("synthetic")).unwrap();
    assert!(json.get("chain").is_none() && json.get("stations").is_none());
}

#[test]
fn bildkette_task_stats_frame_timing_frame_and_announcement() {
    for request in [
        FrameTimingRequest::start(),
        FrameTimingRequest::probe(1_234),
        FrameTimingRequest {
            on: false,
            probe_us: None,
        },
    ] {
        let frame = ViewerFrame::FrameTiming(request);
        let (tag, payload) = encode_viewer_frame(&frame).unwrap();
        assert_eq!(tag, FRAME_TIMING);
        assert_eq!(decode_viewer_frame(tag, &payload).unwrap(), frame);
    }
    assert_eq!(
        serde_json::to_string(&FrameTimingRequest::start()).unwrap(),
        r#"{"on":true}"#
    );
    // The tag is free among the control frames (`wire.rs`, `RATE_CEILING`, `SET_DECODERS`).
    assert_ne!(
        FRAME_TIMING,
        crate::screen::core::wire_control::RATE_CEILING
    );
    assert_ne!(
        FRAME_TIMING,
        crate::screen::core::set_decoders::SET_DECODERS
    );
    assert_eq!(FRAME_TIMING_VERSION, 1);
}

#[test]
fn bildkette_task_stats_capture_facts_cross_the_process_boundary() {
    for path in CapturePath::ALL {
        for fallback in CaptureFallback::ALL.map(Some).into_iter().chain([None]) {
            for flag in [None, Some(false), Some(true)] {
                let facts = CaptureFacts {
                    path: Some(path),
                    fallback,
                    rate_capped: flag,
                    cursor_embedded: flag.map(|on| !on),
                    on_gpu: flag,
                };
                assert_eq!(CaptureFacts::unpack(facts.pack()), facts);
            }
        }
    }
    assert_eq!(CaptureFacts::unpack(0), CaptureFacts::default());
    // An unknown code of a newer process reads as „not reported“.
    assert_eq!(CaptureFacts::unpack(0xff).path, None);
    // Newer facts keep what they do not know.
    let merged = CaptureFacts {
        path: Some(CapturePath::Xshm),
        ..CaptureFacts::default()
    }
    .merged(CaptureFacts {
        on_gpu: Some(false),
        ..CaptureFacts::default()
    });
    assert_eq!(merged.path, Some(CapturePath::Xshm));
    assert_eq!(merged.on_gpu, Some(false));

    let mark = StreamMark {
        answer: 1,
        frame_id: 2,
        capture_us: 3,
        encode_us: 4,
        dropped: 5,
        handed_us: 6,
        facts: CaptureFacts {
            path: Some(CapturePath::WindowsGraphicsCapture),
            fallback: Some(CaptureFallback::DxgiUnsupported),
            rate_capped: Some(true),
            cursor_embedded: None,
            on_gpu: Some(true),
        }
        .pack(),
        readbacks: 8,
        convert_us: 9,
    };
    assert_eq!(StreamMark::decode(&mark.encode()).unwrap(), mark);
}

#[test]
fn bildkette_task_stats_limits_name_the_holding_station() {
    let base = rates();
    assert_eq!(base.fps_limit(), "viewer");
    let slow = RateFacts {
        capacity_fps: Some(40),
        ..base
    };
    assert_eq!(slow.fps_limit(), "encode_time");
    let still = RateFacts {
        sent_fps: 10,
        ..base
    };
    assert_eq!(still.fps_limit(), "source");
    let pushed = RateFacts {
        back_pressure: true,
        ..still
    };
    assert_eq!(pushed.fps_limit(), "link");

    assert_eq!(base.rate_limit(), "ceiling");
    let codec = RateFacts {
        user_ceiling_bps: None,
        ..base
    };
    assert_eq!(codec.rate_limit(), "codec_ceiling");
    let line = RateFacts {
        target_bps: 5_000_000,
        ..base
    };
    assert_eq!(line.rate_limit(), "link");

    // Read-backs count from the session's start; a CPU picture is `cpu`.
    let mut meter = ChainMeter::default();
    meter.set_readbacks(10);
    meter.set_readbacks(13);
    meter.set_facts(CaptureFacts {
        on_gpu: Some(false),
        ..CaptureFacts::default()
    });
    let report = meter.report(&base);
    assert_eq!(report.readbacks, Some(3));
    assert_eq!(report.convert.as_deref(), Some("cpu"));
    assert_eq!(report.capture_path, None, "nothing reported: absent");
}
