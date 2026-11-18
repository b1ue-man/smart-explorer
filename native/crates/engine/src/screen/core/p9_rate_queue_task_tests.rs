//! Acceptance of plan 2026-11-17 P9 (user: a window moved in circles with
//! AV1 piles up until the picture hangs; the lossless tiles stay at 5 Mbit/s
//! on a line that carries 84; H.264 must never sit at 800 kbit/s): the
//! queue guard follows what the line delivers and always lets two frames
//! through, refused frames lower a target above the line, and the honour of
//! an encoder is judged over the interval its frames really have.
use super::*;
use crate::screen::core::pacing::{guard_bytes, PaceDecision, Pacer, SentFrame};
use crate::screen::core::rate_control::honour::{FrameInterval, HonourMeter, HonourVerdict};
use crate::screen::core::rate_control::model::frame_budget_bytes;
use crate::screen::core::rate_control::{AckSample, RateController};

fn sent(frame_id: u64, bytes: u64, sent_us: u64, keyframe: bool) -> SentFrame {
    SentFrame {
        gop_id: 1,
        frame_id,
        bytes,
        sent_us,
        keyframe,
    }
}

#[test]
fn p9_rate_task_queue_guard_follows_delivery_and_keeps_two_frames() {
    // The line's rate over the round-trip floor plus two frames.
    assert_eq!(
        queue_cap_bytes(30_000_000, 10_000, 100_000, PathKind::Direct),
        237_500
    );
    // On the relay never above its guard.
    assert_eq!(
        queue_cap_bytes(30_000_000, 10_000, 670_000, PathKind::Relay),
        RELAY_GUARD_BYTES
    );
    assert_eq!(frames_allowance(100_000, PathKind::Relay), 200_000);
    // Lossless 2880 x 1800 frames of 670 kB, target 26 Mbit/s, LAN 2 ms:
    // the old guard (target over the round trip + 100 ms, 331 500 bytes)
    // held one frame in flight, so every frame waited for the previous
    // acknowledgement (4.1 frames/s measured on 2026-11-17).
    let mut old = Pacer::default();
    old.decide(0, 26_000_000, PathKind::Direct, 2_000);
    old.on_frame_sent(sent(1, 670_000, 0, true));
    assert_eq!(
        old.decide(30_000, 26_000_000, PathKind::Direct, 2_000),
        PaceDecision::DropBeforeEncode
    );
    // The new guard lets the next frame be encoded while one is on the line.
    let mut pacer = Pacer::default();
    pacer.set_queue(84_000_000, 2_000);
    assert_eq!(
        pacer.decide(0, 26_000_000, PathKind::Direct, 2_000),
        PaceDecision::Encode { keyframe: true }
    );
    pacer.on_frame_sent(sent(1, 670_000, 0, true));
    assert_eq!(
        pacer.decide(30_000, 26_000_000, PathKind::Direct, 2_000),
        PaceDecision::Encode { keyframe: false }
    );
    pacer.on_frame_sent(sent(2, 670_000, 30_000, false));
    // A third frame would queue behind two: it is skipped before encoding.
    assert_eq!(
        pacer.decide(60_000, 26_000_000, PathKind::Direct, 2_000),
        PaceDecision::DropBeforeEncode
    );
    assert!(pacer.on_ack(1, 1).is_some());
    assert_eq!(
        pacer.decide(90_000, 26_000_000, PathKind::Direct, 2_000),
        PaceDecision::Encode { keyframe: false }
    );
    // AV1 at a target of 84 Mbit/s on a line of 30 with frames of 350 kB:
    // the old guard queued three frames (285 ms at 30 Mbit/s), the new one
    // at most two, i.e. one beyond the frame on the line.
    assert!(guard_bytes(84_000_000, PathKind::Direct, 2_000) >= 3 * 350_000);
    let mut video = Pacer::default();
    video.set_queue(30_000_000, 2_000);
    video.on_frame_sent(sent(1, 350_000, 0, false));
    assert!(video.guard(84_000_000, PathKind::Direct, 2_000) < 3 * 350_000);
}

#[test]
fn p9_rate_task_delivery_rate_measures_the_line() {
    // Windows of the extremes: the largest and smallest value within it.
    let mut largest = Extremum::max(1_000);
    largest.push(0, 5);
    largest.push(10, 3);
    assert_eq!(largest.value(10), Some(5));
    assert_eq!(largest.value(1_005), Some(3));
    assert_eq!(largest.value(2_011), None);
    let mut smallest = Extremum::min(1_000);
    smallest.push(0, 5);
    smallest.push(10, 7);
    assert_eq!(smallest.value(10), Some(5));
    assert_eq!(smallest.value(1_005), Some(7));
    // Frames acknowledged before the next one is sent measure at most the
    // line, never the line itself (`link_bps` stays empty).
    let mut idle = DeliveryRate::default();
    for index in 0..5u64 {
        let at = index * 33_333;
        idle.on_sent(10_000, at);
        assert!(idle.on_acked(at, at + 5_000).is_some());
    }
    assert_eq!(idle.link_bps(200_000), None);
    assert_eq!(idle.rate_bps(200_000), Some(16_000_000));
    // An unknown frame gives no sample.
    assert_eq!(idle.on_acked(1, 300_000), None);
    idle.reset();
    assert_eq!(idle.rate_bps(300_000), None);
}

/// Six frames of 350 kB at 30 frames/s on a line of 30 Mbit/s: each takes
/// 93.3 ms, so they queue behind each other; acknowledgements 10 ms after
/// the line finished a frame.
fn congested(rate: &mut RateController) {
    let mut events: Vec<(u64, Option<u64>)> = Vec::new();
    for index in 0..6u64 {
        events.push((index * 33_333, None));
        events.push((10_000 + (index + 1) * 93_333, Some(index * 33_333)));
    }
    events.sort_unstable();
    for (at, ack) in events {
        match ack {
            None => rate.on_frame_sent(350_000, at),
            Some(send_us) => {
                let sample = AckSample {
                    send_us,
                    recv_us: at,
                    bytes: 350_000,
                    keyframe: false,
                };
                rate.on_frame_acked(sample, at);
            }
        }
    }
}

#[test]
fn p9_rate_task_refused_frames_lower_a_target_above_the_line() {
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_stream_start(84_000_000);
    rate.set_timing(10_000, 33_333);
    assert_eq!(rate.target_bps(), 84_000_000);
    congested(&mut rate);
    let link = rate.link_rate_bps(600_000).expect("frames queued");
    assert!((29_000_000..=31_000_000).contains(&link), "{link}");
    assert_eq!(rate.base_rtt_us(600_000), Some(10_000));
    // The guard measured the line before the target sank.
    assert!(rate.queue_rate_bps(600_000) <= link);
    // A refused frame: the encoder gets 0.85 × what the line delivered.
    rate.on_back_pressure(600_000);
    let lowered = rate.target_bps();
    assert_eq!(lowered, (0.85 * link as f64).round() as u64);
    // Once below the line, refusals lower nothing more.
    rate.on_back_pressure(700_000);
    assert_eq!(rate.target_bps(), lowered);
    assert_eq!(rate.queue_rate_bps(700_000), lowered);
    // A sender whose small frames never queued knows no line rate: a
    // refusal (a key frame in flight) keeps its target, never the content's.
    let mut light = RateController::new(PathKind::Direct, 0);
    light.set_stream_start(84_000_000);
    light.set_timing(10_000, 33_333);
    for index in 0..5u64 {
        let at = index * 33_333;
        light.on_frame_sent(10_000, at);
        let sample = AckSample {
            send_us: at,
            recv_us: at + 5_000,
            bytes: 10_000,
            keyframe: false,
        };
        light.on_frame_acked(sample, at + 5_000);
    }
    light.on_back_pressure(200_000);
    assert_eq!(light.target_bps(), 84_000_000);
}

#[test]
fn p9_rate_task_honour_budget_counts_the_delivered_interval() {
    // Without frames the open rate counts; faster frames never raise it.
    let mut interval = FrameInterval::default();
    assert_eq!(interval.budget_fps(60), 60);
    for index in 0..10u64 {
        interval.on_frame(index * 10_000);
    }
    assert_eq!(interval.budget_fps(60), 60);
    // An encoder opened for 60 frames/s that gets 15 and keeps the target
    // over the longer interval: the old budget saw a ratio of 4 and cut the
    // rate to a quarter (3.2 Mbit/s of H.264 became 800 kbit/s).
    const TARGET: u64 = 6_000_000;
    let bytes = frame_budget_bytes(TARGET, 15);
    let mut old = HonourMeter::correcting();
    for _ in 0..60 {
        old.on_frame(bytes, false, TARGET, TARGET, 60);
    }
    assert_eq!(old.verdict(), HonourVerdict::Corrected);
    assert_eq!(old.commanded_bps(TARGET), TARGET / 4);
    let mut rate = RateController::new(PathKind::Direct, 0);
    for index in 0..120u64 {
        rate.on_encode_at(bytes, 20_000, index * 66_666);
        rate.on_encoded(bytes, false, TARGET, rate.encoder_bps(), 60, "amf");
    }
    assert_eq!(rate.honour(), HonourVerdict::Honoured);
    assert_eq!(rate.encoder_bps(), rate.target_bps());
}

#[test]
fn p9_rate_task_dropped_pictures_mark_every_tile() {
    // The lossless encoder looks only at the tiles the capture's rectangles
    // touched; a picture the host process dropped before encoding takes its
    // rectangles with it, so every drop site notes it and the next frame
    // looks at every tile.
    let encoder = include_str!("admitted_tiles_encoder.rs").replace("\r\n", "\n");
    assert!(encoder.contains("self.dirty.note(image.size(), image.image().regions());"));
    assert!(encoder.contains("if full || drops != self.drops_seen {"));
    // The unmarked tiles are skipped in the parallel look (P9 tiles-parallel).
    let look = include_str!("tiles_parallel.rs").replace("\r\n", "\n");
    assert!(look.contains("if !full && !*mark {"));
    let capture = include_str!("../os/shared/host_worker_capture_stage.rs");
    assert_eq!(capture.matches("note_dropped_picture();").count(), 1);
    let clock = include_str!("../os/shared/host_worker_clock.rs");
    assert_eq!(clock.matches("note_dropped_picture();").count(), 3);
}
