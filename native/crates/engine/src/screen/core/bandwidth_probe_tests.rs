use super::*;
use crate::screen::core::frame_timing::ChainReport;
use crate::screen::core::wire::{read_frame, write_frame, MAX_CONTROL_BYTES};
use crate::screen::core::wire_control::{decode_viewer_frame, encode_viewer_frame, ViewerFrame};
use crate::screen::core::wire_hello::tests::{client_hello, server_hello};

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn result() -> ProbeResult {
    ProbeResult {
        probe_id: 7,
        bps: 48_000_000,
        bytes: 3_000_000,
        duration_us: 512_000,
    }
}

/// User 2026-11-25: „Übertragung“ and „Neu messen“ travel as `SET_DELIVERY`
/// (58) and `REMEASURE` (59), the viewer's count as `PROBE_RESULT` (60);
/// each has its own budget, refuses what it does not know, and a peer
/// before them keeps working: an old host's hello has no `delivery_v1`, an
/// old viewer's hello no `delivery` and no `bandwidth_probe`, an old
/// host's `STATS` no measured rate and no delivery.
#[test]
fn probe_task_frames_and_hellos_round_trip_and_old_peers_keep_working() {
    let frames = [
        (
            ViewerFrame::SetDelivery(Delivery::Realtime),
            SET_DELIVERY,
            vec![0],
        ),
        (
            ViewerFrame::SetDelivery(Delivery::Reliable),
            SET_DELIVERY,
            vec![1],
        ),
        (ViewerFrame::Remeasure, REMEASURE, b"{}".to_vec()),
        (
            ViewerFrame::ProbeResult(result()),
            PROBE_RESULT,
            result().encode(),
        ),
    ];
    for (frame, tag, payload) in frames {
        assert_eq!(encode_viewer_frame(&frame).unwrap(), (tag, payload.clone()));
        assert_eq!(decode_viewer_frame(tag, &payload).unwrap(), frame);
        let mut wire = Vec::new();
        run(write_frame(&mut wire, tag, &payload)).unwrap();
        let read = run(read_frame(&mut wire.as_slice(), MAX_CONTROL_BYTES));
        assert_eq!(read.unwrap(), (tag, payload));
    }
    assert_eq!(result().encode().len(), PROBE_RESULT_BYTES);
    // Unknown deliveries, other lengths, a result without its probe and a
    // payload above the tag's budget are refused.
    for (tag, payload) in [
        (SET_DELIVERY, vec![2]),
        (SET_DELIVERY, vec![0, 1]),
        (SET_DELIVERY, Vec::new()),
        (REMEASURE, b"[]".to_vec()),
        (PROBE_RESULT, vec![0; PROBE_RESULT_BYTES - 1]),
        (
            PROBE_RESULT,
            ProbeResult {
                probe_id: 0,
                ..result()
            }
            .encode(),
        ),
        (REMEASURE, vec![b' '; REMEASURE_BYTES + 1]),
    ] {
        assert!(
            decode_viewer_frame(tag, &payload).is_err(),
            "{tag} {payload:?}"
        );
    }
    let zero = ViewerFrame::ProbeResult(ProbeResult {
        probe_id: 0,
        ..result()
    });
    assert!(encode_viewer_frame(&zero).is_err());

    // The host announces `delivery_v1`; an old host's hello has none.
    let server = server_hello();
    assert!(server.delivery_v1 && delivery_negotiated(&server));
    let mut old_server = serde_json::to_value(&server).unwrap();
    old_server.as_object_mut().unwrap().remove("delivery_v1");
    let old_server: ScreenServerHello = serde_json::from_value(old_server).unwrap();
    assert!(!delivery_negotiated(&old_server));
    assert!(serde_json::to_value(&old_server)
        .unwrap()
        .get("delivery_v1")
        .is_none());

    // The viewer's fields: absent from an old hello and then not written.
    let old = client_hello();
    assert_eq!((old.delivery, old.bandwidth_probe), (None, None));
    let json = serde_json::to_value(&old).unwrap();
    assert!(json.get("delivery").is_none() && json.get("bandwidth_probe").is_none());
    let mut new = old.clone();
    new.mode = SessionMode::View;
    new.delivery = Some(Delivery::Reliable);
    assert_eq!(viewer_streams(&mut new, false), VIEWER_UNI_STREAMS + 1);
    assert_eq!(new.bandwidth_probe, Some(BANDWIDTH_PROBE_V1));
    let json = serde_json::to_value(&new).unwrap();
    assert_eq!(json["delivery"], "reliable");
    assert_eq!(json["bandwidth_probe"], 1);
    assert_eq!(
        serde_json::from_value::<ScreenClientHello>(json).unwrap(),
        new
    );
    // A probe session takes no stream, a metadata channel no probe.
    let mut probe = old.clone();
    probe.mode = SessionMode::Probe;
    assert_eq!(viewer_streams(&mut probe, false), 0);
    let mut metadata = old.clone();
    metadata.mode = SessionMode::View;
    assert_eq!(viewer_streams(&mut metadata, true), VIEWER_UNI_STREAMS);
    assert_eq!(metadata.bandwidth_probe, None);

    // `STATS`: the chain of an old host has neither field.
    let old_chain: ChainReport = serde_json::from_str(r#"{"rate_limit":"ramp"}"#).unwrap();
    assert_eq!(
        (
            old_chain.measured_kbps,
            old_chain.measured_age_ms,
            old_chain.delivery
        ),
        (None, None, None)
    );
    assert!(serde_json::to_value(&old_chain)
        .unwrap()
        .get("delivery")
        .is_none());
    let chain = ChainReport {
        rate_limit: Some("measured".into()),
        measured_kbps: Some(48_000),
        measured_age_ms: Some(1_500),
        delivery: Some(Delivery::Reliable),
        ..ChainReport::default()
    };
    let json = serde_json::to_value(&chain).unwrap();
    assert_eq!(json["measured_kbps"], 48_000);
    assert_eq!(json["delivery"], "reliable");
    assert_eq!(serde_json::from_value::<ChainReport>(json).unwrap(), chain);
}

/// The probe stream's head: `SCRP`, version, id, duration; 10 round trips
/// within 500 ms and 1.5 s; a head of another version, without id or
/// duration is refused, a longer duration is capped.
#[test]
fn probe_task_head_round_trip_and_duration() {
    assert_eq!(probe_duration_us(1_000), MIN_PROBE_US);
    assert_eq!(probe_duration_us(80_000), 800_000);
    assert_eq!(probe_duration_us(400_000), MAX_PROBE_US);
    assert_eq!(probe_duration_us(u64::MAX), MAX_PROBE_US);
    let head = ProbeHead::for_rtt(9, 80_000);
    assert_eq!(
        (head.probe_id, head.duration_ms, head.duration_us()),
        (9, 800, 800_000)
    );
    let bytes = head.encode();
    assert_eq!(&bytes[..4], b"SCRP");
    assert_eq!(bytes.len(), PROBE_HEAD_BYTES);
    let rest: [u8; PROBE_HEAD_BYTES - 4] = bytes[4..].try_into().unwrap();
    assert_eq!(ProbeHead::decode_after_magic(&rest).unwrap(), head);
    let mut other = rest;
    other[0] = PROBE_VERSION + 1;
    assert!(ProbeHead::decode_after_magic(&other).is_err());
    for head in [
        ProbeHead {
            probe_id: 0,
            duration_ms: 500,
        },
        ProbeHead {
            probe_id: 3,
            duration_ms: 0,
        },
    ] {
        let rest: [u8; PROBE_HEAD_BYTES - 4] = head.encode()[4..].try_into().unwrap();
        assert!(ProbeHead::decode_after_magic(&rest).is_err());
    }
    let long = ProbeHead {
        probe_id: 3,
        duration_ms: u16::MAX,
    };
    let rest: [u8; PROBE_HEAD_BYTES - 4] = long.encode()[4..].try_into().unwrap();
    assert_eq!(
        ProbeHead::decode_after_magic(&rest).unwrap().duration_ms,
        MAX_HEAD_MS
    );
    // The filler is one static buffer of a fixed size.
    assert_eq!(FILLER.len(), FILLER_BYTES);
}

/// The viewer's count: every byte that reached the connection (probe and
/// pictures alike) per window of at least 50 ms, the largest window's rate;
/// a late tick lengthens its window instead of raising its rate; no
/// complete window gives 0; the meter keeps no samples.
#[test]
fn probe_task_meter_takes_the_largest_window_and_counts_pictures() {
    // The congestion window ramps up: 50, 100, 200, 300 and 300 KB of probe
    // bytes per 50-ms window, and pictures add 50 KB per window throughout.
    let mut meter = ProbeMeter::new(0, 1_000);
    let mut total = 1_000u64;
    for (index, probe) in [50_000u64, 100_000, 200_000, 300_000, 300_000]
        .iter()
        .enumerate()
    {
        total += probe + 50_000;
        meter.sample((index as u64 + 1) * WINDOW_US, total);
    }
    let answer = meter.result(4);
    // 350 KB in 50 ms: 56 Mbit/s, pictures included.
    assert_eq!(answer.bps, 56_000_000);
    assert_eq!(answer.bytes, total - 1_000);
    assert_eq!((answer.probe_id, answer.duration_us), (4, 5 * WINDOW_US));

    // A tick 30 ms late: that window lasts 80 ms; its bytes count over it.
    let mut late = ProbeMeter::new(0, 0);
    late.sample(80_000, 400_000);
    assert_eq!(late.result(1).bps, 40_000_000);
    // Samples within a window accumulate; an early one closes nothing.
    let mut early = ProbeMeter::new(0, 0);
    early.sample(20_000, 100_000);
    early.sample(40_000, 200_000);
    assert_eq!(early.result(1).bps, 0);
    early.sample(50_000, 250_000);
    assert_eq!(early.result(1).bps, 40_000_000);
    // A total or clock that goes back counts as the last one.
    early.sample(60_000, 10);
    early.sample(10_000, 260_000);
    assert_eq!(early.result(1).bytes, 260_000);
    assert_eq!(early.result(1).duration_us, 60_000);
    // Bounded memory: a long count keeps the same few numbers.
    let mut long = ProbeMeter::new(0, 0);
    for step in 1..=200_000u64 {
        long.sample(step * 1_000, step * 1_250);
    }
    assert_eq!(long.result(1).bps, 10_000_000);
    assert!(std::mem::size_of::<ProbeMeter>() <= 7 * 8);
}
