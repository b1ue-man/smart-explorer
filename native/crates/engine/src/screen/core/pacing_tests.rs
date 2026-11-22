use super::*;
use crate::screen::core::rate_control::{MAX_BITRATE_BPS, MIN_BITRATE_BPS};

const TARGET: u64 = 6_000_000;
const RTT_US: u64 = 10_000;

fn sent(gop_id: u64, frame_id: u64, bytes: u64, sent_us: u64, keyframe: bool) -> SentFrame {
    SentFrame {
        gop_id,
        frame_id,
        bytes,
        sent_us,
        keyframe,
    }
}

#[test]
fn screen_task_pacing_drops_before_encode() {
    // 6 Mbit/s over a 10 ms round trip allows 110 ms of data: 82 500 bytes.
    assert_eq!(guard_bytes(TARGET, PathKind::Direct, RTT_US), 82_500);
    let mut pacer = Pacer::default();
    let direct = PathKind::Direct;
    assert_eq!(
        pacer.decide(0, TARGET, direct, RTT_US),
        PaceDecision::Encode { keyframe: true }
    );
    pacer.on_frame_sent(sent(1, 1, 60_000, 0, true));
    assert_eq!(
        pacer.decide(33_000, TARGET, direct, RTT_US),
        PaceDecision::Encode { keyframe: false }
    );
    pacer.on_frame_sent(sent(1, 2, 30_000, 33_000, false));
    // 90 000 unacknowledged plus an expected 30 000: the captures are
    // skipped before encoding, nothing encoded is ever thrown away.
    assert_eq!(
        pacer.decide(66_000, TARGET, direct, RTT_US),
        PaceDecision::DropBeforeEncode
    );
    assert_eq!(
        pacer.decide(99_000, TARGET, direct, RTT_US),
        PaceDecision::DropBeforeEncode
    );
    assert_eq!(pacer.dropped_before_encode(), 2);
    assert_eq!(pacer.unacked_bytes(), 90_000);
    // The acknowledgement of the key frame frees its bytes.
    let acked = pacer.on_ack(1, 1).unwrap();
    assert_eq!(
        (acked.bytes, acked.sent_us, acked.keyframe),
        (60_000, 0, true)
    );
    assert_eq!(pacer.unacked_bytes(), 30_000);
    assert_eq!(
        pacer.decide(100_000, TARGET, direct, RTT_US),
        PaceDecision::Encode { keyframe: false }
    );
    // An acknowledgement covers every earlier frame of its stream.
    pacer.on_frame_sent(sent(1, 3, 30_000, 100_000, false));
    assert_eq!(pacer.on_ack(1, 3).map(|frame| frame.frame_id), Some(3));
    assert_eq!((pacer.unacked_bytes(), pacer.frames_in_flight()), (0, 0));
    assert_eq!(pacer.on_ack(1, 2), None);
    // With nothing in flight a frame always goes out, even a key frame far
    // larger than the guard of the smallest target.
    pacer.request_keyframe();
    pacer.request_keyframe();
    assert_eq!(
        pacer.decide(133_000, MIN_BITRATE_BPS, direct, RTT_US),
        PaceDecision::Encode { keyframe: true }
    );
    pacer.on_frame_sent(sent(2, 4, 500_000, 133_000, true));
    assert!(!pacer.keyframe_due(), "two requests made one key frame");
    assert_eq!(
        pacer.decide(166_000, MIN_BITRATE_BPS, direct, RTT_US),
        PaceDecision::DropBeforeEncode
    );
}

#[test]
fn screen_task_pacing_relay_guard_below_relay_budget() {
    // The guard keeps 256 KiB of the relay's 1 MiB per destination free.
    assert_eq!(RELAY_GUARD_BYTES + 256 * 1024, RELAY_QUEUE_BYTES);
    for target in [
        MIN_BITRATE_BPS,
        2_000_000,
        TARGET,
        50_000_000,
        MAX_BITRATE_BPS,
    ] {
        for rtt in [0, 1_000, 50_000, 300_000, 1_000_000, 60_000_000] {
            let relay = guard_bytes(target, PathKind::Relay, rtt);
            assert!(relay <= RELAY_GUARD_BYTES, "{target} bit/s, {rtt} µs");
            assert!(relay <= guard_bytes(target, PathKind::Direct, rtt));
        }
    }
    // Below the cap the formula applies; the cap binds only on the relay.
    assert_eq!(guard_bytes(2_000_000, PathKind::Relay, 50_000), 37_500);
    assert_eq!(
        guard_bytes(50_000_000, PathKind::Direct, 100_000),
        1_250_000
    );
    assert_eq!(
        guard_bytes(50_000_000, PathKind::Relay, 100_000),
        RELAY_GUARD_BYTES
    );

    // 50 Mbit/s on the relay without acknowledgements for 0.9 s: frames of
    // 64 KiB stop at exactly 768 KiB in flight, below the relay budget.
    let mut pacer = Pacer::default();
    let (mut frames, mut highest) = (0u64, 0u64);
    for now_us in (0..900_000).step_by(1_000) {
        match pacer.decide(now_us, 50_000_000, PathKind::Relay, 100_000) {
            PaceDecision::Encode { keyframe } => {
                frames += 1;
                pacer.on_frame_sent(sent(1, frames, 64 * 1024, now_us, keyframe));
            }
            PaceDecision::DropBeforeEncode => {}
            PaceDecision::ResetStream => panic!("no frame is older than one second"),
        }
        highest = highest.max(pacer.unacked_bytes());
    }
    assert_eq!(frames, 12);
    assert_eq!(highest, RELAY_GUARD_BYTES);
    assert!(highest < RELAY_QUEUE_BYTES);
}

#[test]
fn screen_task_pacing_resets_stale_gop() {
    let direct = PathKind::Direct;
    let mut pacer = Pacer::default();
    pacer.on_frame_sent(sent(1, 1, 50_000, 0, true));
    assert_eq!(
        pacer.decide(500_000, TARGET, direct, RTT_US),
        PaceDecision::Encode { keyframe: false }
    );
    pacer.on_frame_sent(sent(1, 2, 10_000, 500_000, false));
    // Exactly one second is still in time; beyond it the stream resets.
    assert_ne!(
        pacer.decide(1_000_000, TARGET, direct, RTT_US),
        PaceDecision::ResetStream
    );
    assert_eq!(
        pacer.decide(1_000_001, TARGET, direct, RTT_US),
        PaceDecision::ResetStream
    );
    pacer.on_stream_reset();
    assert_eq!((pacer.unacked_bytes(), pacer.frames_in_flight()), (0, 0));
    assert_eq!(pacer.resets(), 1);
    // The next frame is a key frame on the new stream; late
    // acknowledgements of the old one are ignored.
    assert_eq!(
        pacer.decide(1_000_002, TARGET, direct, RTT_US),
        PaceDecision::Encode { keyframe: true }
    );
    assert_eq!(pacer.on_ack(1, 2), None);

    // Too many frames in flight reset the stream regardless of their age.
    let mut pacer = Pacer::default();
    for frame_id in 0..MAX_IN_FLIGHT_FRAMES as u64 {
        let decision = pacer.decide(frame_id, MAX_BITRATE_BPS, direct, STALE_STREAM_US);
        assert!(matches!(decision, PaceDecision::Encode { .. }));
        pacer.on_frame_sent(sent(1, frame_id, 100, frame_id, frame_id == 0));
    }
    assert_eq!(
        pacer.decide(2_000, MAX_BITRATE_BPS, direct, STALE_STREAM_US),
        PaceDecision::ResetStream
    );
}

#[test]
fn screen_task_pacing_frame_clock_cadence() {
    // A 60 Hz source with ±2 ms jitter yields exactly 30 frames/s.
    let mut clock = FrameClock::new(30);
    let jitter = [0i64, 2_000, -2_000, 1_000, -1_000];
    let taken = (0..120i64)
        .filter(|&frame| {
            let now = (frame * 16_667 + jitter[frame as usize % 5]).max(0) as u64;
            clock.take(now)
        })
        .count();
    assert_eq!(taken, 60);
    // 60 → 45 frames/s and 60 → 60 frames/s.
    let mut clock = FrameClock::new(45);
    assert_eq!(
        (0..120).filter(|&frame| clock.take(frame * 16_667)).count(),
        90
    );
    let mut clock = FrameClock::new(60);
    assert_eq!(
        (0..120).filter(|&frame| clock.take(frame * 16_667)).count(),
        120
    );
    // After a stall the cadence starts over instead of bursting.
    let mut clock = FrameClock::new(30);
    assert!(clock.take(0));
    let after: Vec<bool> = (0..6)
        .map(|frame| clock.take(500_000 + frame * 16_667))
        .collect();
    assert_eq!(after, [true, false, true, false, true, false]);
    // The rate is clamped to 1..=240 frames/s.
    assert_eq!(FrameClock::new(0).interval_us(), 1_000_000);
    assert_eq!(FrameClock::new(1_000).interval_us(), 1_000_000 / 240);
}

/// Review 2026-11-19 (Gemini 3/3, round 5 G2/G5): the viewer's
/// acknowledgement leaves when a picture is read off the network; what it
/// shows (`PRESENTED`) paces the pictures. A picture received and not shown
/// for more than `VIEWER_LAG_US` holds the next one back while the viewer
/// shows pictures, one picture follows every `PRESENTED`, a viewer that
/// keeps up is never held, and no stream is reset for it.
#[test]
fn review1119_task_pictures_follow_what_the_viewer_shows() {
    use super::{PRESENTED_FRESH_US, PRESENTED_HOLD_MAX_US, VIEWER_LAG_US};
    let mut pacer = Pacer::default();
    let frame = |frame_id: u64| SentFrame {
        gop_id: 1,
        frame_id,
        bytes: 1_000,
        sent_us: 0,
        keyframe: frame_id == 1,
    };
    // 240 pictures a second, received at once, shown 20 ms later: never held.
    for id in 1..=20 {
        pacer.on_frame_sent(frame(id));
        let at = id * 4_166;
        pacer.on_ack(1, id);
        pacer.on_received(id, at);
        pacer.on_presented(id.saturating_sub(5).max(1), at);
        assert!(!pacer.viewer_behind(at + 20_000), "{id}");
    }
    // A viewer without `PRESENTED` (older ones, the bench) is never held.
    let mut old = Pacer::default();
    old.on_frame_sent(frame(1));
    old.on_ack(1, 1);
    old.on_received(1, 0);
    assert!(!old.viewer_behind(10 * VIEWER_LAG_US));
    // The viewer shows picture 20 and then falls behind: 21 waits unshown.
    pacer.on_presented(20, 100_000);
    pacer.on_frame_sent(frame(21));
    pacer.on_ack(1, 21);
    pacer.on_received(21, 100_000);
    let late = 100_000 + VIEWER_LAG_US + 1;
    assert!(pacer.viewer_behind(late));
    // Its next `PRESENTED` lets one picture follow, which uses the credit.
    pacer.on_presented(20, late);
    assert!(!pacer.viewer_behind(late));
    pacer.on_frame_sent(frame(22));
    assert!(pacer.viewer_behind(late));
    // Three of its own gaps between two `PRESENTED` (0.25 s) are less than
    // `PRESENTED_FRESH_US`: after that it is not waited for (a gate, a path
    // without `PRESENTED`), never one picture a second.
    assert!(pacer.viewer_behind(late + PRESENTED_FRESH_US));
    assert!(!pacer.viewer_behind(late + PRESENTED_FRESH_US + 1));
    // A slow viewer (4K software decoding) is held for 3 of its gaps, at most
    // `PRESENTED_HOLD_MAX_US`.
    let mut slow = Pacer::default();
    slow.on_received(1, 0);
    slow.on_received(2, 0);
    slow.on_presented(1, 100_000);
    slow.on_received(3, 100_000);
    slow.on_presented(2, 2_100_000);
    slow.on_frame_sent(frame(4));
    assert!(slow.viewer_behind(2_100_000 + 4_000_000));
    assert!(!slow.viewer_behind(2_100_000 + PRESENTED_HOLD_MAX_US + 1));
    // Round 9 F6: a still screen between two `PRESENTED` (nothing waited)
    // is no slow viewer: the hold stays `PRESENTED_FRESH_US`.
    let mut idle = Pacer::default();
    idle.on_received(1, 0);
    idle.on_presented(1, 16_000);
    idle.on_received(2, 10_000_000);
    idle.on_presented(2, 10_016_000);
    idle.on_received(3, 10_020_000);
    idle.on_frame_sent(frame(4));
    assert!(idle.viewer_behind(10_016_000 + PRESENTED_FRESH_US));
    assert!(!idle.viewer_behind(10_016_000 + PRESENTED_FRESH_US + 1));
    // It never resets a stream: the decision stays.
    assert_eq!(
        pacer.decide(late, 6_000_000, PathKind::Direct, 1_000),
        PaceDecision::Encode { keyframe: false }
    );
}
