use super::super::{AckSample, PathKind, RateController};
use super::RateLimit;
use crate::screen::core::delivery::Delivery;
use crate::screen::core::pacing::{PaceDecision, Pacer, SentFrame, RELAY_GUARD_BYTES};

const FRAME_US: u64 = 16_666;

fn sent(frame_id: u64, bytes: u64, sent_us: u64) -> SentFrame {
    SentFrame {
        gop_id: 1,
        frame_id,
        bytes,
        sent_us,
        keyframe: frame_id == 1,
    }
}

/// User 2026-11-25: „ich WILL NICHT DASS DU DROSSELST“. The unthrottled
/// controller gives the encoder the viewer's rate, else the measured one,
/// else the stream's table; overuse, loss and stale streams change nothing.
#[test]
fn unthrottled_task_no_estimate_lowers_the_rate() {
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_unthrottled(true);
    rate.set_stream_start(35_000_000);
    rate.set_codec_ceiling(200_000_000);
    rate.set_timing(10_000, FRAME_US);
    assert_eq!(rate.target_bps(), 35_000_000);
    assert_eq!(rate.limit(), RateLimit::Ramp);
    // A queue that grows 6 ms per frame (persistent overuse), heavy loss
    // and a stale stream: the regulated controller lowered each time.
    for index in 0..60u64 {
        let send_us = index * FRAME_US;
        rate.on_frame_sent(20_000, send_us);
        let recv_us = send_us + 10_000 + index * 6_000;
        let sample = AckSample {
            send_us,
            recv_us,
            bytes: 20_000,
            keyframe: false,
        };
        rate.on_frame_acked(sample, recv_us);
    }
    let _ = rate.on_loss_counters(1_000, 0);
    let _ = rate.on_loss_counters(2_000, 400);
    rate.on_stale_stream(2_000_000, 100_000);
    rate.on_back_pressure(2_000_000);
    assert_eq!(rate.target_bps(), 35_000_000);
    // A measurement replaces it, up and down.
    rate.set_measured(80_000_000);
    assert_eq!(rate.target_bps(), 80_000_000);
    assert_eq!(rate.limit(), RateLimit::Measured);
    assert_eq!(rate.limit().key(), "measured");
    rate.set_measured(12_000_000);
    assert_eq!(rate.target_bps(), 12_000_000);
    // The viewer's own „Datenrate“ is the rate; the codec pairing bounds it.
    rate.set_ceiling(Some(50_000_000));
    assert_eq!(rate.target_bps(), 50_000_000);
    assert_eq!(rate.limit(), RateLimit::Ceiling);
    rate.set_ceiling(None);
    rate.set_measured(900_000_000);
    assert_eq!(rate.target_bps(), 200_000_000);
    // The regulated controller (tests of the rate model) is unchanged.
    assert!(!RateController::new(PathKind::Direct, 0).unthrottled());
}

/// Unthrottled, no queue guard drops a picture before encoding; „Echtzeit“
/// gives a late picture up after the stale time, „Zuverlässig“ never.
#[test]
fn unthrottled_task_pacer_drops_nothing_and_delivery_decides_late_pictures() {
    let direct = PathKind::Direct;
    let mut realtime = Pacer::default();
    realtime.set_unthrottled(true);
    realtime.set_timing(10_000, FRAME_US);
    assert_eq!(realtime.delivery(), Delivery::Realtime);
    // Ten pictures of 1 MB in flight at a target of 1 Mbit/s: the guard of
    // the regulated pacer dropped every one after the first.
    for frame_id in 1..=10u64 {
        let now = frame_id * FRAME_US;
        assert!(matches!(
            realtime.decide(now, 1_000_000, direct, 10_000),
            PaceDecision::Encode { .. } | PaceDecision::ResetStream
        ));
        realtime.on_frame_sent(sent(frame_id, 1_000_000, now));
    }
    assert_eq!(realtime.dropped_before_encode(), 0);
    // Late beyond the stale time (at 1 Gbit/s the 10 MB drain in 80 ms):
    // „Echtzeit“ gives the stream up.
    assert_eq!(
        realtime.decide(60_000_000, 1_000_000_000, direct, 10_000),
        PaceDecision::ResetStream
    );
    let mut reliable = realtime.clone();
    reliable.set_delivery(Delivery::Reliable);
    assert!(matches!(
        reliable.decide(60_000_000, 1_000_000_000, direct, 10_000),
        PaceDecision::Encode { .. }
    ));
    // The relay drops above its queue per destination: its guard stays.
    let mut relay = Pacer::default();
    relay.set_unthrottled(true);
    relay.set_delivery(Delivery::Reliable);
    relay.on_frame_sent(sent(1, RELAY_GUARD_BYTES, 0));
    relay.on_frame_sent(sent(2, 400_000, FRAME_US));
    assert_eq!(
        relay.decide(2 * FRAME_US, 100_000_000, PathKind::Relay, 10_000),
        PaceDecision::DropBeforeEncode
    );
}
