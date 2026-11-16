//! BK0 (plan 2026-11-14 „Bildkette“): the host's stations in `se screen
//! bench`, the old fields with a host without `frame_timing_v1`, and the
//! real source.
use std::collections::BTreeMap;

use super::host::HostTiming;
use super::*;
use crate::screen::core::frame_timing::{
    ChainReport, FrameTimingRequest, HostStations, SkewedUs, StationUs, FRAME_TIMING_VERSION,
};
use crate::screen::core::metrics::ClockSync;
use crate::screen::core::wire_control::{HostStats, Pong};

fn station(p50_us: u64, p95_us: u64) -> Option<StationUs> {
    Some(StationUs { p50_us, p95_us })
}

fn stats(frame_timing: Option<u8>, stations: Option<HostStations>) -> HostStats {
    HostStats {
        target_kbps: 6000,
        sent_kbps: 5800,
        fps: 30,
        path: PathKind::Direct,
        keyframes: 1,
        dropped_before_encode: 0,
        stream_resets: 0,
        unacked_bytes: 0,
        frame_timing,
        chain: frame_timing.map(|_| ChainReport {
            capture_path: Some("test_picture".into()),
            ..ChainReport::default()
        }),
        stations,
    }
}

/// A clock whose host runs 1 s ahead of the viewer (no round trip).
fn clock() -> ClockSync {
    let mut clock = ClockSync::default();
    let pong = Pong {
        sequence: 1,
        ping_sent_us: 10_000_000,
        received_us: 11_000_000,
        sent_us: 11_000_000,
    };
    clock.on_pong(&pong, 10_000_000);
    assert_eq!(clock.offset_us(), Some(1_000_000));
    clock
}

fn report() -> BenchReport {
    let mut stages_ms = BTreeMap::new();
    stages_ms.insert("decode".to_string(), StageMs { p50: 2.0, p95: 3.0 });
    BenchReport {
        codec: "tiles".into(),
        encoder: "tiles".into(),
        decoder: "tiles".into(),
        path: "direct".into(),
        frames_sent: 30,
        frames_shown: 30,
        fps_mean: 30.0,
        fps_min: 29.0,
        latency_ms: LatencyMs {
            p50: 10.0,
            p95: 12.0,
            max: 15.0,
        },
        bitrate_kbps: BitrateKbps {
            mean: 900.0,
            max: 1000.0,
        },
        keyframes: 1,
        recoveries: Vec::new(),
        corrupt_frames: 0,
        static_psnr_db: None,
        stages_ms,
        source: None,
        host_timing: None,
        host_chain: None,
    }
}

#[test]
fn bildkette_task_bench_stations_complete_with_synthetic_source() {
    let mut host = HostTiming::default();
    // The first announcement starts the station times.
    let first = host.on_stats(stats(Some(FRAME_TIMING_VERSION), None), 5);
    assert_eq!(
        first,
        Some(ViewerFrame::FrameTiming(FrameTimingRequest::start()))
    );
    let stations = HostStations {
        capture: station(1_000, 2_000),
        convert: None,
        encode: station(4_000, 6_000),
        worker_handoff: station(100, 300),
        send: station(500, 900),
        tick_jitter: station(50, 400),
        input_inject: station(200, 250),
        // Viewer arrival minus host send; the host runs 1 s ahead.
        network_raw: Some(SkewedUs {
            p50_us: -998_000,
            p95_us: -996_000,
        }),
        // Host handling minus the viewer's probe time.
        input_raw: Some(SkewedUs {
            p50_us: 1_003_000,
            p95_us: 1_005_000,
        }),
    };
    // Every later report brings an input probe with the viewer clock.
    let later = host.on_stats(stats(Some(FRAME_TIMING_VERSION), Some(stations)), 77);
    assert_eq!(
        later,
        Some(ViewerFrame::FrameTiming(FrameTimingRequest::probe(77)))
    );
    let mut report = report();
    host.finish(&clock(), &mut report);
    assert_eq!(report.host_timing.as_deref(), Some(HOST_TIMING_REPORTED));
    assert_eq!(
        report.host_chain.as_ref().unwrap().capture_path.as_deref(),
        Some("test_picture")
    );
    let expect = [
        ("capture", 1.0, 2.0),
        ("encode", 4.0, 6.0),
        ("worker_handoff", 0.1, 0.3),
        ("send", 0.5, 0.9),
        ("tick_jitter", 0.05, 0.4),
        ("input_inject", 0.2, 0.25),
        ("network", 2.0, 4.0),
        ("input", 3.0, 5.0),
        ("decode", 2.0, 3.0),
    ];
    for (name, p50, p95) in expect {
        assert_eq!(report.stages_ms[name], StageMs { p50, p95 }, "{name}");
    }
    assert!(!report.stages_ms.contains_key("convert"));
    // The old fields keep their names and values.
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["frames_shown"], 30);
    assert_eq!(json["latency_ms"]["p95"], 12.0);
    assert_eq!(json["host_timing"], "reported");
    assert!(json.get("source").is_none());
}

#[test]
fn bildkette_task_bench_old_host_keeps_the_old_fields() {
    let mut host = HostTiming::default();
    // A host of v0.5.196: no announcement, so no `FRAME_TIMING` goes out.
    assert_eq!(host.on_stats(stats(None, None), 5), None);
    assert_eq!(host.on_stats(stats(None, None), 6), None);
    let before = serde_json::to_value(report()).unwrap();
    let mut report = report();
    host.finish(&clock(), &mut report);
    assert_eq!(report.host_timing.as_deref(), Some(HOST_TIMING_UNSUPPORTED));
    assert_eq!(report.stages_ms.len(), 1, "only the viewer's station");
    assert!(report.host_chain.is_none());
    let mut after = serde_json::to_value(&report).unwrap();
    after.as_object_mut().unwrap().remove("host_timing");
    assert_eq!(after, before, "every other field unchanged");
    // The station times need the clock offset: without one they stay out.
    let mut host = HostTiming::default();
    host.on_stats(stats(Some(1), None), 1);
    let stations = HostStations {
        network_raw: Some(SkewedUs {
            p50_us: 5,
            p95_us: 6,
        }),
        ..HostStations::default()
    };
    host.on_stats(stats(Some(1), Some(stations)), 2);
    let mut report = report();
    host.finish(&ClockSync::default(), &mut report);
    assert!(!report.stages_ms.contains_key("network"));
}

#[test]
fn bildkette_task_bench_old_reader_and_old_report() {
    // A report of a bench before BK0 reads with the new fields absent.
    let mut old = serde_json::to_value(report()).unwrap();
    old.as_object_mut().unwrap().remove("stages_ms");
    let read: BenchReport = serde_json::from_value(old).unwrap();
    assert_eq!(read.source, None);
    assert_eq!(read.host_timing, None);
    assert_eq!(read.host_chain, None);
    // The test picture leaves the new fields out of the JSON entirely.
    let json = serde_json::to_value(report()).unwrap();
    for key in ["source", "host_timing", "host_chain"] {
        assert!(json.get(key).is_none(), "{key}");
    }
    // With `--source` the report names it.
    let mut real = report();
    real.source = Some("monitor:0".into());
    let json = serde_json::to_value(&real).unwrap();
    assert_eq!(json["source"], "monitor:0");
}
