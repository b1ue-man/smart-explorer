//! Acceptance of plan 2026-11-14 „Bildkette“ package BK1 (B-02, B-03, B-05,
//! B-06, B-08, B-09) at the rate control and its sources; the growth tests
//! follow the bounded growth of plan 2026-11-17 (a sender that fills the
//! target drives it, `bildkette_rate_model_task_tests.rs`).
use super::*;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;

const FRAME_US: u64 = 16_667;
const DELAY_US: u64 = 5_000;
/// 8 MB per frame: 3.84 Gbit/s at 60 frames/s, a sender that fills every
/// target of these tests (plan 2026-11-17 bounds the growth by delivery).
const BIG_FRAME: u64 = 8_000_000;
/// The stream's table start of the app-limited test (1080p60 H.264).
const TABLE: u64 = 20_000_000;

/// Sends frame `index` of a 60 frames/s stream with `bytes` and acknowledges
/// it `delay_us` later; the target before the frame and the signal.
fn frame(rate: &mut RateController, index: u64, bytes: u64, delay_us: u64) -> (u64, DelaySignal) {
    let before = rate.target_bps();
    let send_us = index * FRAME_US;
    rate.on_frame_sent(bytes, send_us);
    let sample = AckSample {
        send_us,
        recv_us: send_us + delay_us,
        bytes,
        keyframe: false,
    };
    (before, rate.on_frame_acked(sample, send_us + delay_us))
}

fn peer(name: &str) -> String {
    format!("bildkette-task-{name}-{}", std::process::id())
}

#[test]
fn bildkette_task_rate_frame_rate_is_independent_of_the_bit_rate() {
    // The cadence is the wanted rate, at most what the encoding carries; the
    // bit rate does not enter it (v0.5.196: 6 Mbit/s gave 2160p 14 frames/s).
    let uhd = FrameSize::new(3840, 2160);
    assert_eq!(quality_for(START_DIRECT_BPS, uhd, 60).fps, 14);
    for target in [
        MIN_BITRATE_BPS,
        START_RELAY_BPS,
        START_DIRECT_BPS,
        100_000_000,
    ] {
        let mut rate = RateController::new(PathKind::Direct, 0);
        rate.set_ceiling(Some(target));
        assert_eq!(cadence_fps(60, rate.encode_capacity_fps()), 60);
        assert_eq!(cadence_fps(144, rate.encode_capacity_fps()), 144);
    }
    // A measured encoding time limits it: a median of 10 ms carries 100
    // frames/s.
    let mut rate = RateController::new(PathKind::Direct, 0);
    for index in 0..100u64 {
        let elapsed = if index % 20 == 0 { 4_000 } else { 10_000 };
        rate.on_encode_at(50_000, elapsed, index * 10_000);
    }
    assert_eq!(rate.encode_capacity_fps(), Some(100));
    assert_eq!(cadence_fps(144, rate.encode_capacity_fps()), 100);
    assert_eq!(cadence_fps(60, rate.encode_capacity_fps()), 60);
    // 2026-11-24: one frame in ten taking 100 ms (key frames, a GPU stall)
    // keeps the cadence at what the typical frame carries; the 95th
    // percentile made it 10 frames/s.
    let mut stalls = RateController::new(PathKind::Direct, 0);
    for index in 0..100u64 {
        let elapsed = if index % 10 == 0 { 100_000 } else { 10_000 };
        stalls.on_encode_at(50_000, elapsed, index * 10_000);
    }
    assert_eq!(stalls.encode_capacity_fps(), Some(100));
    // Older samples than the window leave it.
    let mut timing = EncodeTiming::default();
    timing.push(0, 50_000);
    timing.push(ENCODE_WINDOW_US + 1, 5_000);
    assert_eq!(timing.p95_us(), Some(5_000));
}

#[test]
fn bildkette_task_rate_cadence_change_keeps_the_encoder() {
    use crate::screen::core::pacing::codec_cap;
    // The encoder opens with the highest allowed rate; a sinking cadence
    // leaves that rate, so the running encoder stays.
    let size = FrameSize::new(1920, 1080);
    let open = codec_cap(144, size, None).unwrap();
    assert_eq!(open, 144);
    for capacity in [Some(30), Some(90), None] {
        let cadence = cadence_fps(144, capacity);
        assert!(cadence <= open);
        assert_eq!(codec_cap(144, size, None), Some(open));
    }
    // The macroblock rate of the pairing still bounds the open rate.
    assert_eq!(codec_cap(144, size, Some(245_760)), Some(30));
    // The pipeline restarts an encoder only for a higher allowed rate, and
    // the once-a-second cadence step never ends the encoder.
    // A Windows checkout has CRLF line ends.
    let produce = include_str!("host_pipeline_produce.rs").replace("\r\n", "\n");
    assert!(produce.contains("open_fps > self.codec_fps"));
    assert!(!produce.contains("let changed = self.codec_fps != fps;"));
    let mode = include_str!("host_mode.rs").replace("\r\n", "\n");
    let second = &mode[mode.find("pub fn second").unwrap()..];
    let second = &second[..second.find("\n    }\n").unwrap()];
    assert!(!second.contains("drop_encoder"));
    assert!(!second.contains("adjust_quality"));
    assert!(mode.contains("same_but_rate"));
}

#[test]
fn bildkette_task_rate_ramp_doubles_per_round_trip_until_overuse() {
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_timing(20_000, FRAME_US);
    assert!(rate.ramping());
    // A sender that fills the target (8 MB per frame, 3.84 Gbit/s offered)
    // with constant delay: the target doubles once per max(RTT, frame
    // interval), within twice what was delivered (slow start).
    let mut changes = Vec::new();
    let mut index = 0;
    while changes.len() < 10 && index < 200 {
        let (before, _) = frame(&mut rate, index, BIG_FRAME, DELAY_US);
        let after = rate.target_bps();
        if after != before {
            changes.push((index, before, after));
        }
        index += 1;
    }
    assert_eq!(changes.len(), 10, "{changes:?}");
    let delivered = rate.receive_rate_bps().unwrap();
    for (_, before, after) in &changes {
        assert_eq!(*after, before * 2, "{changes:?}");
        assert!(*after <= 2 * delivered, "{changes:?}");
    }
    for pair in changes.windows(2) {
        let gap_us = (pair[1].0 - pair[0].0) * FRAME_US;
        assert!(gap_us >= 20_000, "at most once per interval: {changes:?}");
    }
    assert!(rate.target_bps() > START_DIRECT_BPS * 1000);
    // A queue builds up: 6 ms more delay per frame. The first persistent
    // overuse ends the ramp; the sender filled the target, so it sinks to
    // 0.85 × the receive rate (an application-limited one: 0.85 × its own
    // target, `bildkette_task_rate_app_limited_overuse_never_drops_to_
    // content_rate`).
    let mut decreased = false;
    for step in 1..40 {
        let (_, signal) = frame(&mut rate, index, BIG_FRAME, DELAY_US + step * 6_000);
        index += 1;
        if signal == DelaySignal::Decrease {
            decreased = true;
            break;
        }
    }
    assert!(decreased, "persistent overuse lowers the target");
    assert!(!rate.ramping());
    let receive = rate.receive_rate_bps().unwrap();
    assert_eq!(rate.target_bps(), (0.85 * receive as f64).round() as u64);
}

#[test]
fn bildkette_task_rate_receive_rate_does_not_hold_an_application_limited_target() {
    // Plan 2026-11-17 replaces 5.3a's unbounded growth: an
    // application-limited sender is not held at its receive rate (v0.5.196
    // held it at 1.25 × that), it grows back to the stream's table start,
    // but never above the larger of the table and 1.5 × what was delivered
    // + 10 kbit/s.
    let name = peer("app-limited");
    probe::remember(&name, PathKind::Direct, START_DIRECT_BPS);
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_stream_start(TABLE);
    rate.bind_peer(&name);
    assert!(!rate.ramping(), "a remembered rate ends the ramp");
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    rate.set_timing(20_000, FRAME_US);
    for index in 0..60 {
        frame(&mut rate, index, 1_000, DELAY_US);
    }
    let receive = rate.receive_rate_bps().unwrap();
    assert_eq!(rate.target_bps(), TABLE);
    assert!(rate.target_bps() as f64 > 1.5 * receive as f64 + 10_000.0);
    // Back pressure (a frame dropped before encoding) holds it.
    let mut held = RateController::new(PathKind::Direct, 0);
    held.set_stream_start(TABLE);
    held.bind_peer(&name);
    held.set_timing(20_000, FRAME_US);
    held.on_back_pressure(0);
    for index in 0..10 {
        frame(&mut held, index, 1_000, DELAY_US);
    }
    assert_eq!(held.target_bps(), START_DIRECT_BPS);
    // A sender that fills the target is bounded by what was delivered.
    let mut full = RateController::new(PathKind::Direct, 0);
    full.set_stream_start(1_000_000);
    full.bind_peer(&name);
    full.set_timing(20_000, FRAME_US);
    let filling = START_DIRECT_BPS / 60 / 8;
    for index in 0..60 {
        frame(&mut full, index, filling, DELAY_US);
    }
    let delivered = full.receive_rate_bps().unwrap();
    assert!(full.target_bps() > START_DIRECT_BPS);
    assert!(full.target_bps() as f64 <= 1.5 * delivered as f64 + 10_000.0 + 1.0);
}

#[test]
fn bildkette_task_rate_ceiling_comes_from_encoder_and_decoder() {
    // H.264 Table A-1 MaxBR × cpbBrVclFactor of the level both report.
    let cb = H264Profile::ConstrainedBaseline;
    assert_eq!(avc_max_bitrate_bps(Some(983_040), cb), 240_000_000);
    assert_eq!(
        avc_max_bitrate_bps(Some(983_040), H264Profile::High),
        300_000_000
    );
    assert_eq!(avc_max_bitrate_bps(Some(245_760), cb), 50_000_000);
    assert_eq!(avc_max_bitrate_bps(Some(16_711_680), cb), 800_000_000);
    assert_eq!(avc_max_bitrate_bps(None, H264Profile::High), 1_000_000_000);
    let choice = CodecChoice {
        codec: CodecKind::H264,
        profile: Some(H264Profile::High),
        encoder_hardware: true,
        decoder_hardware: true,
        max_size: FrameSize::new(4096, 2304),
        max_pixels: 4096 * 2304,
        max_macroblocks_per_second: Some(2_073_600),
    };
    let size = FrameSize::new(1920, 1080);
    assert_eq!(codec_ceiling_bps(&choice, size, 60), 300_000_000);
    let tiles = CodecChoice {
        codec: CodecKind::Tiles,
        profile: None,
        max_macroblocks_per_second: None,
        ..choice
    };
    assert_eq!(codec_ceiling_bps(&tiles, size, 60), 1920 * 1080 * 32 * 60);
    // No fixed 240 Mbit/s: a sender that fills it ramps to the pairing's
    // ceiling.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_codec_ceiling(avc_max_bitrate_bps(None, H264Profile::High));
    rate.set_timing(1_000, FRAME_US);
    for index in 0..80 {
        frame(&mut rate, index, BIG_FRAME, DELAY_US);
    }
    assert_eq!(rate.target_bps(), 1_000_000_000);
    assert!(rate.target_bps() > MAX_BITRATE_BPS);
}

#[test]
fn bildkette_task_rate_user_ceiling_bounds_the_target() {
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_timing(1_000, FRAME_US);
    rate.set_ceiling(Some(10_000_000));
    assert_eq!(rate.user_ceiling_bps(), Some(10_000_000));
    for index in 0..40 {
        frame(&mut rate, index, BIG_FRAME, DELAY_US);
    }
    assert_eq!(rate.target_bps(), 10_000_000);
    // A lower ceiling lowers the target at once; automatic lets it grow.
    rate.set_ceiling(Some(3_000_000));
    assert_eq!(rate.target_bps(), 3_000_000);
    rate.set_ceiling(Some(1));
    assert_eq!(rate.target_bps(), MIN_BITRATE_BPS);
    rate.set_ceiling(None);
    // Automatic returns to the link estimate, which the viewer's ceiling
    // never cut (plan 2026-11-17).
    for index in 40..80 {
        frame(&mut rate, index, 20_000, DELAY_US);
    }
    assert!(rate.target_bps() > 3_000_000);
}

#[test]
fn bildkette_task_rate_start_value_per_peer_and_path() {
    let name = peer("memo");
    probe::remember(&name, PathKind::Relay, 9_000_000);
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.bind_peer(&name);
    assert!(rate.ramping(), "nothing remembered for the direct path");
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    rate.set_path(PathKind::Relay);
    assert!(!rate.ramping());
    assert_eq!(rate.target_bps(), 9_000_000);
    // A ramping target is no measurement and is not remembered.
    let fresh = peer("fresh");
    let mut ramp = RateController::new(PathKind::Direct, 0);
    ramp.bind_peer(&fresh);
    ramp.remember();
    assert_eq!(probe::remembered(&fresh, PathKind::Direct), None);
    // After the first persistent overuse it is.
    ramp.set_timing(20_000, FRAME_US);
    let mut index = 0;
    while index < 20 {
        frame(&mut ramp, index, 1_000, DELAY_US);
        index += 1;
    }
    for step in 1..40 {
        let (_, signal) = frame(&mut ramp, index, 1_000, DELAY_US + step * 6_000);
        index += 1;
        if signal == DelaySignal::Decrease {
            break;
        }
    }
    assert!(!ramp.ramping());
    ramp.remember();
    assert_eq!(
        probe::remembered(&fresh, PathKind::Direct),
        Some(ramp.target_bps())
    );
    // Review 2026-11-19 (Gemini 9): a target the user's ceiling holds is no
    // measurement of the path and does not replace the remembered rate.
    let measured = ramp.target_bps();
    ramp.set_ceiling(Some(measured / 2));
    ramp.remember();
    assert_eq!(probe::remembered(&fresh, PathKind::Direct), Some(measured));
}

#[test]
fn bildkette_task_rate_windows_tick_uses_a_high_resolution_timer() {
    // Text test, every platform: the Windows session tick (B-08).
    let timer = include_str!("../os/windows/timer_resolution.rs");
    for needle in [
        "CREATE_WAITABLE_TIMER_HIGH_RESOLUTION",
        "timeGetDevCaps",
        "timeBeginPeriod(caps.period_min)",
        "timeEndPeriod(self.0)",
        "PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION",
    ] {
        assert!(timer.contains(needle), "{needle}");
    }
    let os = include_str!("../os/mod.rs");
    assert!(os.contains("#[path = \"windows/timer_resolution.rs\"]"));
    let session = include_str!("host_session.rs");
    assert!(session.contains("clock::SessionClock::start(max_fps)"));
    assert!(!session.contains("tokio::time::interval(tick)"));
    let clock = include_str!("host_session_clock.rs");
    assert!(clock.contains("SessionTimer::start(tick)"));
}
