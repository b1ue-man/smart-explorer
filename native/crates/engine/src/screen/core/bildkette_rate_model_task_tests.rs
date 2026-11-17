//! Acceptance of plan 2026-11-17 (user: latency below 20 ms like Shadow, a
//! sharp picture, „av1 ist zu viel. punkt.“, „h.264 capped bei 800kbit/s …
//! nicht ok“) at the rate model: the start table, the bounded growth, the
//! decrease that never falls to the content rate and the named limits. The
//! honour of the encoders: `bildkette_rate_honour_task_tests.rs`.
use super::model::{classify, frame_budget_bytes, session_start_bps, start_table_bps, LimitFacts};
use super::*;
use crate::screen::core::chain_meter::{ChainMeter, RateFacts};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;

pub(super) const FRAME_US: u64 = 16_667;
const DELAY_US: u64 = 5_000;
const MBIT: u64 = 1_000_000;

/// Sends frame `index` of a 60 frames/s stream with `bytes`, acknowledged
/// `delay_us` later; the signal.
pub(super) fn frame(
    rate: &mut RateController,
    index: u64,
    bytes: u64,
    delay_us: u64,
) -> DelaySignal {
    let send_us = index * FRAME_US;
    rate.on_frame_sent(bytes, send_us);
    let sample = AckSample {
        send_us,
        recv_us: send_us + delay_us,
        bytes,
        keyframe: false,
    };
    rate.on_frame_acked(sample, send_us + delay_us)
}

/// Frames from `index` on with 6 ms more delay each until the persistent
/// overuse lowers the target; the index after it.
fn overuse(rate: &mut RateController, mut index: u64, bytes: u64) -> u64 {
    for step in 1..60 {
        let signal = frame(rate, index, bytes, DELAY_US + step * 6_000);
        index += 1;
        if signal == DelaySignal::Decrease {
            return index;
        }
    }
    panic!("no persistent overuse");
}

/// `frames` frames of `bytes` with a constant delay from index 0 on.
fn steady(rate: &mut RateController, frames: u64, bytes: u64) {
    for index in 0..frames {
        frame(rate, index, bytes, DELAY_US);
    }
}

fn peer(name: &str) -> String {
    format!("bildkette-rate-model-{name}-{}", std::process::id())
}

/// A controller on `path` that remembered `bps` for its peer.
fn remembered(name: &str, path: PathKind, bps: u64) -> RateController {
    let name = peer(name);
    probe::remember(&name, path, bps);
    let mut rate = RateController::new(path, 0);
    rate.bind_peer(&name);
    rate.set_timing(20_000, FRAME_US);
    rate
}

#[test]
fn bildkette_task_rate_model_start_follows_resolution_fps_codec() {
    use CodecKind::{Av1, Hevc, Tiles, H264};
    // Moonlight's rows at 30 frames/s, linear between them, never above the
    // last; × fps/30 up to 60, × √(fps/60) × 2 above; the codec's share and
    // 4:4:4; never below the floor (1 frame/s does not round to nothing).
    let cases = [
        (1280, 720, 30, H264, false, 5 * MBIT),
        (1920, 1080, 30, H264, false, 10 * MBIT),
        (2560, 1440, 30, H264, false, 20 * MBIT),
        (3840, 2160, 30, H264, false, 40 * MBIT),
        (1600, 900, 30, H264, false, 7_250_000),
        (5120, 2880, 30, H264, false, 40 * MBIT),
        (1920, 1080, 60, H264, false, 20 * MBIT),
        (3840, 2160, 60, H264, false, 80 * MBIT),
        (1920, 1080, 120, H264, false, 28_284_271),
        (1920, 1080, 60, Hevc, false, 14 * MBIT),
        (1920, 1080, 60, Av1, false, 12 * MBIT),
        (1920, 1080, 60, Av1, true, 24 * MBIT),
        (1920, 1080, 60, Tiles, false, 20 * MBIT),
        (320, 240, 1, H264, false, MIN_BITRATE_BPS),
    ];
    for (width, height, fps, codec, chroma, bps) in cases {
        let size = FrameSize::new(width, height);
        let start = start_table_bps(size, fps, codec, chroma);
        assert_eq!(start, bps, "{size:?} {fps} {codec:?}");
    }
    // Session start: a remembered rate wins, the relay guards the table.
    let table = Some(20 * MBIT);
    let starts = [
        (table, None, false, 5 * MBIT, 5 * MBIT),
        (table, None, false, u64::MAX, 20 * MBIT),
        (table, None, true, u64::MAX, START_RELAY_BPS),
        (table, Some(9 * MBIT), true, u64::MAX, 9 * MBIT),
        (None, None, false, u64::MAX, START_DIRECT_BPS),
    ];
    for (table, known, relay, ceiling, bps) in starts {
        let start = if relay {
            START_RELAY_BPS
        } else {
            START_DIRECT_BPS
        };
        assert_eq!(session_start_bps(table, known, relay, start, ceiling), bps);
    }
    // The controller starts at the table before its first frame, then keeps.
    let mut direct = RateController::new(PathKind::Direct, 0);
    direct.set_stream_start(20 * MBIT);
    assert_eq!(direct.target_bps(), 20 * MBIT);
    direct.on_frame_sent(1_000, 0);
    direct.set_stream_start(40 * MBIT);
    assert_eq!(direct.target_bps(), 20 * MBIT);
    let mut relay = RateController::new(PathKind::Relay, 0);
    relay.set_stream_start(20 * MBIT);
    assert_eq!(relay.target_bps(), START_RELAY_BPS);
    let mut capped = RateController::new(PathKind::Direct, 0);
    capped.set_stream_start(20 * MBIT);
    capped.set_codec_ceiling(5 * MBIT);
    assert_eq!(capped.target_bps(), 5 * MBIT);
    // A remembered rate replaces the table, on the relay above its guard.
    let mut known = remembered("start", PathKind::Relay, 9 * MBIT);
    known.set_stream_start(20 * MBIT);
    assert_eq!(known.target_bps(), 9 * MBIT);
    assert_eq!(frame_budget_bytes(20 * MBIT, 60), 41_666);
}

#[test]
fn bildkette_task_rate_app_limited_growth_is_bounded_by_delivery() {
    // AV1 1080p60: table 12 Mbit/s, pairing ceiling 160 Mbit/s. A desktop
    // that sends 1 kB pictures grew ×1.25 per interval to the ceiling (84
    // Mbit/s and more in the user's session); now it stays at the table.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_codec_ceiling(160 * MBIT);
    rate.set_stream_start(12 * MBIT);
    rate.set_timing(20_000, FRAME_US);
    steady(&mut rate, 600, 1_000);
    assert_eq!(rate.target_bps(), 12 * MBIT);
    // A still screen (one picture every 2 s, fewer than two in the window)
    // no longer blocks the growth: from a remembered 4 Mbit/s back to the
    // table, and not beyond it.
    let mut still = remembered("still", PathKind::Direct, 4 * MBIT);
    still.set_stream_start(20 * MBIT);
    assert_eq!(still.target_bps(), 4 * MBIT);
    for second in 0..12u64 {
        let send_us = second * 2_000_000;
        still.on_frame_sent(1_000, send_us);
        let sample = AckSample {
            send_us,
            recv_us: send_us + DELAY_US,
            bytes: 1_000,
            keyframe: false,
        };
        still.on_frame_acked(sample, send_us + DELAY_US);
    }
    assert_eq!(still.target_bps(), 20 * MBIT);
    // A sender that fills the target grows to at most 1.5 × delivery +
    // 10 kbit/s and by at most 1.25 per interval.
    let mut full = remembered("filling", PathKind::Direct, 6 * MBIT);
    full.set_stream_start(MBIT);
    let mut before = full.target_bps();
    for index in 0..120 {
        frame(&mut full, index, 12_500, DELAY_US);
        let after = full.target_bps();
        assert!(after as f64 <= before as f64 * model::INTERVAL_GROWTH + 1.0);
        before = after;
    }
    let delivered = full.receive_rate_bps().unwrap();
    assert!(full.target_bps() > 6 * MBIT);
    assert!(full.target_bps() as f64 <= model::delivery_cap_bps(delivered) + 1.0);
}

#[test]
fn bildkette_task_rate_app_limited_overuse_never_drops_to_content_rate() {
    // 1 kB pictures (480 kbit/s of content) at a target of 20 Mbit/s: a
    // persistent overuse lowers the target to 0.85 × itself, not to 0.85 ×
    // the receive rate (the 800 kbit/s of the user's H.264 session).
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_stream_start(20 * MBIT);
    rate.set_timing(20_000, FRAME_US);
    steady(&mut rate, 20, 1_000);
    assert_eq!(rate.target_bps(), 20 * MBIT);
    let index = overuse(&mut rate, 20, 1_000);
    assert!(!rate.ramping());
    assert_eq!(rate.target_bps(), 17 * MBIT);
    assert!(rate.target_bps() > 10 * rate.receive_rate_bps().unwrap());
    // The overuse is back pressure: the next one (v0.5.196 then took the
    // receive rate) lowers it from itself again.
    assert!(rate.back_pressure(index * FRAME_US));
    overuse(&mut rate, index, 1_000);
    assert_eq!(rate.target_bps(), 14_450_000);
    assert!(rate.target_bps() > 10 * rate.receive_rate_bps().unwrap());
    // `decreased` itself: the content rate never enters an app-limited one.
    let lowered = model::decreased(20_000_000.0, true, Some(400_000));
    assert_eq!(lowered, 17_000_000.0);
    let lowered = model::decreased(20_000_000.0, false, Some(10_000_000));
    assert_eq!(lowered, 8_500_000.0);
}

#[test]
fn bildkette_task_rate_limit_names_ramp_content_link_ceilings() {
    let user = Some(10 * MBIT);
    let higher = Some(20 * MBIT);
    let own = Some(8 * MBIT);
    // (viewer, codec, encoder ceiling, ramping, app-limited, corrected).
    let cases = [
        (None, None, None, false, false, false, "link"),
        (None, None, None, false, true, false, "content"),
        (None, None, None, true, true, false, "ramp"),
        (user, None, None, true, false, false, "ceiling"),
        (None, user, None, false, false, false, "codec_ceiling"),
        (higher, user, None, false, false, false, "codec_ceiling"),
        (None, None, own, false, false, false, "encoder_ceiling"),
        (user, None, None, false, false, true, "encoder_ignores"),
    ];
    for (viewer, codec, encoder, ramping, app_limited, corrected, key) in cases {
        let facts = LimitFacts {
            target_bps: 10 * MBIT,
            user_ceiling_bps: viewer,
            codec_ceiling_bps: codec,
            encoder_ceiling_bps: encoder,
            ramping,
            app_limited,
            encoder_corrected: corrected,
        };
        assert_eq!(classify(&facts).key(), key, "{facts:?}");
    }
    // The controller names its own reason; v0.5.196 said `link` for any
    // target below the ceilings.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_stream_start(20 * MBIT);
    assert_eq!(rate.limit(), RateLimit::Ramp);
    rate.set_timing(20_000, FRAME_US);
    steady(&mut rate, 20, 1_000);
    overuse(&mut rate, 20, 1_000);
    assert_eq!(rate.limit(), RateLimit::Content);
    rate.set_ceiling(Some(rate.target_bps()));
    assert_eq!(rate.limit(), RateLimit::Ceiling);
    rate.set_ceiling(None);
    rate.set_codec_ceiling(MBIT);
    assert_eq!(rate.limit(), RateLimit::CodecCeiling);
    let mut line = remembered("link", PathKind::Direct, 6 * MBIT);
    steady(&mut line, 20, 12_500);
    overuse(&mut line, 20, 12_500);
    assert_eq!(line.limit(), RateLimit::Link);
    // `STATS`: the reason, the encoder's rate and its honour.
    let facts = RateFacts {
        target_bps: 5 * MBIT,
        ceiling_bps: 20 * MBIT,
        limit: Some(RateLimit::Content),
        encoder_bps: Some(2_500_000),
        honour: Some(honour::HonourVerdict::Corrected),
        ..RateFacts::default()
    };
    assert_eq!(facts.rate_limit(), "content");
    let mut old = facts;
    old.limit = None;
    assert_eq!(old.rate_limit(), "link", "a pipeline without a reason");
    let report = ChainMeter::default().report(&facts);
    assert_eq!(report.rate_limit.as_deref(), Some("content"));
    assert_eq!(report.encoder_rate_kbps, Some(2_500));
    assert_eq!(report.rate_honour.as_deref(), Some("corrected"));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["encoder_rate_kbps"], 2_500);
    let without = ChainMeter::default().report(&RateFacts::default());
    let json = serde_json::to_value(&without).unwrap();
    assert!(json.get("encoder_rate_kbps").is_none() && json.get("rate_honour").is_none());
}
