//! Task tests of the fix „Diashow“ (2026-11-18, 0.5.200: the tiles at
//! 3 pictures a second at „Ziel 20414“ with 15 resets and 15 full
//! pictures, NVENC HEVC checked again at 50 openings in 2 minutes). Pure:
//! no network, no GPU, no capture.
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;
use crate::screen::core::codec_registry::{DecoderFactory, EncoderFactory};
use crate::screen::core::codec_verdicts::VerdictStore;
use crate::screen::core::codec_verify::Checks;
use crate::screen::core::first_picture::{FirstPictureWatch, FIRST_PICTURE_DEADLINE_US};
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer};
use crate::screen::core::pacing::{Pacer, SentFrame};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::{codec_ceiling_bps, PathKind, RateController};

fn pairing(codec: CodecKind, hardware: bool) -> CodecChoice {
    CodecChoice {
        codec,
        profile: None,
        encoder_hardware: hardware,
        decoder_hardware: false,
        max_size: FrameSize::MAX,
        max_pixels: FrameSize::MAX.pixels(),
        max_macroblocks_per_second: None,
    }
}

/// A rate control of 1080p tiles at 60 pictures a second.
fn tiles_rate() -> RateController {
    let mut rate = RateController::new(PathKind::Direct, 0);
    let size = FrameSize::new(1920, 1080);
    rate.set_codec_ceiling(codec_ceiling_bps(
        &pairing(CodecKind::Tiles, false),
        size,
        60,
    ));
    rate
}

#[test]
fn diashowfix_task_tile_budget_books_once_and_spreads_debt() {
    // 8 Mbit/s: a picture of 1 MB costs one second of the line.
    const RATE: u64 = 8_000_000;
    let mut budget = TileBudget::default();
    // Before the first picture the credit is full.
    assert!(budget.admits(0, RATE));
    // Booked when its send starts (0), not when the send ended (1 s): the
    // debt beyond the credit of 200 ms is paid off after 0.8 s. Booking
    // at the end of the send charged its second twice (next at 1.8 s).
    budget.on_sent(0, 1_000_000, RATE);
    assert!(!budget.admits(799_000, RATE));
    assert!(budget.admits(800_000, RATE));
    assert_eq!(budget.wait_us(0, RATE), 800_000);
    assert_eq!(budget.wait_us(800_000, RATE), 0);
    // The debt is spread over time: over any span the bytes stay within
    // the rate plus the credit plus one picture, and the stream gets the
    // rate (no starving).
    let mut budget = TileBudget::default();
    let (mut bits, mut now) = (0u64, 0u64);
    while now < 10_000_000 {
        if budget.admits(now, RATE) {
            budget.on_sent(now, 300_000, RATE);
            bits += 300_000 * 8;
        }
        now += 16_667;
    }
    let credit = RATE * TILE_BURST_US / 1_000_000;
    assert!(bits <= RATE * 10 + credit + 300_000 * 8, "{bits}");
    assert!(bits + 300_000 * 8 >= RATE * 10, "{bits}");
    // A higher rate pays an old debt at once (congestion over).
    let mut budget = TileBudget::default();
    budget.on_sent(0, 10_000_000, 1_000_000);
    assert!(!budget.admits(1_000_000, 1_000_000));
    assert!(budget.admits(1_000_000, 4_000_000_000));
}

#[test]
fn diashowfix_task_tiles_follow_the_line_without_congestion() {
    let mut rate = tiles_rate();
    // Without a user ceiling and congestion the budget is the ceiling,
    // far above the start target E that an application-limited tile stream
    // never raises: a full picture of 6 MB does not hold the next one back
    // (0.5.199: 137 pictures a second; 0.5.200: a freeze of size/E).
    assert_eq!(rate.tiles_budget_bps(0), rate.ceiling_bps());
    assert!(rate.tiles_budget_bps(0) > 100 * rate.target_bps());
    rate.on_tiles_sent(0, 6_000_000);
    assert!(rate.tiles_admit(16_667));
    // The user ceiling stays a hard ceiling of the tiles.
    rate.set_ceiling(Some(650_000));
    assert_eq!(rate.tiles_budget_bps(20_000), 650_000);
    rate.on_tiles_sent(1_000_000, 1_000_000);
    assert!(!rate.tiles_admit(1_500_000));
    assert!(rate.tiles_admit(13_200_000));
    // A stale stream (the path did not carry the target) holds the tiles
    // at the lowered target for `CONGESTION_HOLD_US`, then the line's
    // queue guard alone again.
    let mut rate = tiles_rate();
    let before = rate.target_bps();
    rate.on_stale_stream(1_000_000, 500_000);
    let lowered = rate.target_bps();
    assert!(lowered < before);
    assert_eq!(rate.tiles_budget_bps(1_000_000), lowered);
    assert_eq!(
        rate.tiles_budget_bps(1_000_000 + CONGESTION_HOLD_US),
        lowered
    );
    assert_eq!(
        rate.tiles_budget_bps(1_000_001 + CONGESTION_HOLD_US),
        rate.ceiling_bps()
    );
    // Loss above 10 % holds them at E until a reading below it.
    let mut rate = tiles_rate();
    let _ = rate.on_loss_counters(1_000, 0);
    let _ = rate.on_loss_counters(2_000, 200);
    assert_eq!(rate.tiles_budget_bps(0), rate.target_bps());
    let _ = rate.on_loss_counters(3_000, 200);
    assert_eq!(rate.tiles_budget_bps(0), rate.ceiling_bps());
    // A persistent delay overuse marks the congestion as well, and the
    // pipeline books a picture when its send starts.
    let control = include_str!("rate_control.rs");
    assert!(control.contains("self.tiles.on_congestion(now_us);"));
    // Review 2026-11-19 (Codex 13): at the writer, the record headers and
    // a cut-off write included.
    let send = include_str!("host_frame_send.rs").replace("\r\n", "\n");
    assert!(send.contains("let send_started_us = now_us();"));
    assert!(send.contains("self.rate.on_stream_sent(send_started_us, booked, video);"));
}

#[test]
fn diashowfix_task_slow_tile_stream_waits_instead_of_reset() {
    // Past its stale time a tile picture is late: dropped before encoding,
    // the target sinks; no reset, no full picture.
    assert_eq!(tile_stale(Some(200_001), 200_000), TileStale::Late);
    assert_eq!(tile_stale(Some(200_000), 200_000), TileStale::Crowded);
    // The frame count alone (many small pictures) is back pressure only.
    assert_eq!(tile_stale(None, 200_000), TileStale::Crowded);
    assert_eq!(tile_stale(Some(10), 200_000), TileStale::Crowded);
    // Only a stream that does not move at all is given up.
    assert_eq!(
        tile_stale(Some(TILE_STUCK_US + 1), u64::MAX),
        TileStale::Stuck
    );
    let mut pacer = Pacer::default();
    assert_eq!(pacer.oldest_age_us(5_000), None);
    pacer.on_frame_sent(SentFrame {
        gop_id: 1,
        frame_id: 1,
        bytes: 3_000_000,
        sent_us: 1_000,
        keyframe: true,
    });
    assert_eq!(pacer.oldest_age_us(5_000), Some(4_000));
    // The pipeline asks before it resets a stale tile stream.
    let produce = include_str!("host_pipeline_produce.rs").replace("\r\n", "\n");
    assert!(produce.contains("self.tiles_stale(now, target)"));
    let wait = include_str!("host_pipeline_tiles.rs");
    assert!(wait.contains("TileStale::Stuck => None,"));
}

/// The body of `fn <name>` in `source` up to its closing brace.
fn body<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source.find(name).expect("function present");
    let rest = &source[start..];
    let end = rest.find("\n    }\n").expect("function end");
    &rest[..end]
}

#[test]
fn diashowfix_task_reset_keeps_the_encoder() {
    let pipeline = include_str!("host_pipeline.rs").replace("\r\n", "\n");
    // A stream reset asks the running encoder for its key frame and opens
    // no new one (each opening ran the codec check and, with NVENC on the
    // measured host, failed it again).
    let reset = body(&pipeline, "fn reset_stream(&mut self)");
    assert!(reset.contains("self.new_group();"));
    assert!(!reset.contains("drop_encoder"));
    assert!(reset.contains("self.kept_encoder = self.running.is_some();"));
    // A changed codec, contract or source still gets a new encoder.
    let invalidate = body(&pipeline, "pub fn invalidate_reference(&mut self)");
    assert!(invalidate.contains("self.reset_stream_and_encoder();"));
    assert!(
        body(&pipeline, "fn reset_stream_and_encoder(&mut self)").contains("self.drop_encoder();")
    );
    // A kept encoder that answers without a key frame gives way to a new
    // one instead of counting against it.
    let produce = include_str!("host_pipeline_produce.rs").replace("\r\n", "\n");
    assert!(produce.contains("if std::mem::take(&mut self.kept_encoder) {"));
    // A viewer's key-frame request keeps the encoder as well.
    let mode = include_str!("host_mode.rs").replace("\r\n", "\n");
    let request = body(&mode, "pub fn request_keyframe(&mut self)");
    assert!(request.contains("self.new_group();") && request.contains("self.reset_stream();"));
    assert!(!request.contains("invalidate_reference"));
}

/// An encoder that never opens (NVENC status 15 on the measured host).
struct Unopenable {
    opened: AtomicUsize,
}

impl EncoderFactory for Unopenable {
    fn name(&self) -> String {
        "nvenc-hevc".into()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::video(CodecKind::Hevc, false, true)]
    }

    fn open(&self, _: &CodecChoice, _: FrameSize, _: u32) -> io::Result<Box<dyn ScreenEncoder>> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("NVENC status 15"))
    }
}

#[test]
fn diashowfix_task_unopened_encoder_is_checked_once_per_process() {
    let checks = Checks::new(VerdictStore::memory());
    let unopenable = Arc::new(Unopenable {
        opened: AtomicUsize::new(0),
    });
    let factory: Arc<dyn EncoderFactory> = unopenable.clone();
    let decoders: Vec<Arc<dyn DecoderFactory>> = Vec::new();
    let choice = pairing(CodecKind::Hevc, true);
    let size = FrameSize::new(256, 144);
    // The opening still decides (as before), the check runs once.
    for _ in 0..50 {
        assert_eq!(checks.admit(&factory, &decoders, &choice, size), Ok(()));
    }
    assert_eq!(unopenable.opened.load(Ordering::SeqCst), 1);
    let key = Checks::encoder_key(factory.as_ref(), &choice, size);
    assert!(checks.unopened(&key));
    // No verdict is stored: another process checks it again.
    assert!(checks.store().get(&key).is_none());
}

#[test]
fn diashowfix_task_first_picture_watch_ignores_idle_time() {
    let av1 = pairing(CodecKind::Av1, true);
    let mut watch = FirstPictureWatch::default();
    // One slow first answer, then a still screen for ten seconds (its
    // answers are „unchanged“ and not counted), then two more: no failure.
    assert!(!watch.on_silent(av1, 0));
    assert!(!watch.on_silent(av1, 10_000_000));
    assert!(!watch.on_silent(av1, 10_100_000));
    assert!(!watch.on_silent(av1, 10_200_000));
    // An encoder that stays silent while it gets pictures still fails once
    // the deadline has passed since the idle time ended.
    let mut now = 10_200_000;
    let failed = loop {
        now += 100_000;
        if watch.on_silent(av1, now) {
            break now;
        }
    };
    assert_eq!(failed, 10_000_000 + FIRST_PICTURE_DEADLINE_US);
}
