//! Acceptance of plan 2026-11-17 (user: "every encoder must provably honour
//! the rate se sets", the lossless mode failing in real sessions): the
//! honour meter and the watch of the encoders, the Media Foundation buffer
//! in bytes, the encoders opening at the session's rate and the stale time
//! of a large lossless key frame on a line slower than the target.
use std::io;

use super::bildkette_rate_model_task_tests::{frame, FRAME_US};
use super::honour::{guard, EncoderRateState, HonourMeter, HonourVerdict, HONOUR_WINDOW};
use super::model::{frame_budget_bytes, mf_buffer_bytes};
use super::*;
use crate::screen::core::host_backend::{EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pacing::{PaceDecision, Pacer, SentFrame};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};

const FPS: u32 = 60;
const SIDE: u32 = 64;
const TARGET: u64 = 6_000_000;

/// An encoder whose frames are `factor` × the budget it is given.
struct Overshooting {
    factor: f64,
    refused: bool,
}

impl ScreenEncoder for Overshooting {
    fn name(&self) -> String {
        "overshooting".into()
    }
    fn size(&self) -> FrameSize {
        FrameSize::new(SIDE, SIDE)
    }
    fn rate_state(&self) -> Option<EncoderRateState> {
        Some(EncoderRateState {
            applied_bps: 0,
            refused: self.refused,
        })
    }
    fn encode(&mut self, _: &BgraImage, key: bool, bps: u64) -> io::Result<EncodedFrame> {
        let bytes = (frame_budget_bytes(bps, FPS) as f64 * self.factor) as usize;
        Ok(EncodedFrame {
            records: vec![vec![0u8; bytes]],
            keyframe: key,
        })
    }
}

fn watched(factor: f64, refused: bool, codec: CodecKind, demote: bool) -> Box<dyn ScreenEncoder> {
    guard(Box::new(Overshooting { factor, refused }), codec, demote)
}

fn request() -> StageRequest {
    let size = FrameSize::new(SIDE, SIDE);
    StageRequest {
        frame_id: 1,
        frame_contract: None,
        visible_rects: None,
        keyframe: false,
        target_bps: TARGET,
        size,
        choice: CodecChoice {
            codec: CodecKind::H264,
            profile: Some(H264Profile::High),
            encoder_hardware: true,
            decoder_hardware: true,
            max_size: size,
            max_pixels: size.pixels(),
            max_macroblocks_per_second: None,
        },
        encoder_epoch: 1,
        fps: FPS,
    }
}

/// Encodes `frames` frames: how many succeeded, and the last error.
fn run(encoder: &mut Box<dyn ScreenEncoder>, frames: usize) -> (usize, Option<String>) {
    let image = BgraImage::new(FrameSize::new(SIDE, SIDE)).unwrap();
    let (mut ok, mut last) = (0, None);
    for _ in 0..frames {
        match encoder.encode_frame(&image, &request()) {
            Ok(_) => ok += 1,
            Err(error) => last = Some(error.to_string()),
        }
    }
    (ok, last)
}

/// One window of frames of `bytes` given `commanded` bit/s; the verdict.
fn window(meter: &mut HonourMeter, bytes: u64, commanded: u64) -> HonourVerdict {
    for _ in 0..HONOUR_WINDOW {
        meter.on_frame(bytes, false, TARGET, commanded, FPS);
    }
    meter.verdict()
}

#[test]
fn bildkette_task_rate_honour_meter_corrects_and_demotes() {
    use HonourVerdict::{Corrected, Honoured, Ignores, Pending};
    let budget = frame_budget_bytes(TARGET, FPS);
    let mut meter = HonourMeter::correcting();
    // Frames below half their budget and key frames say nothing.
    assert_eq!(window(&mut meter, budget / 3, TARGET), Pending);
    meter.on_frame(budget * 9, true, TARGET, TARGET, FPS);
    assert_eq!(meter.verdict(), Pending);
    // (frame size, rate given, verdict, rate given next): twice the budget
    // halves the rate, which the encoder then keeps (the correction holds);
    // frames well below se's budget give a step back; five times is
    // ignoring, and at the floor 1.5 is too.
    let steps = [
        (budget, TARGET, Honoured, TARGET),
        (budget * 2, TARGET, Corrected, 3_000_000),
        (budget, 3_000_000, Honoured, 3_000_000),
        (budget * 6 / 10, 3_000_000, Honoured, 3_750_000),
        (budget * 5, 3_750_000, Ignores, 1_500_000),
        (budget * 2, 1_500_000, Ignores, 1_500_000),
    ];
    for (bytes, given, verdict, next) in steps {
        let judged = window(&mut meter, bytes, given);
        assert_eq!(judged, verdict, "{bytes} at {given}");
        assert_eq!(meter.commanded_bps(TARGET), next, "{bytes} at {given}");
    }
    assert_eq!(meter.state(), Ignores);
    let mut kept = HonourMeter::correcting();
    window(&mut kept, budget * 2, TARGET);
    window(&mut kept, budget, 3_000_000);
    assert_eq!((kept.verdict(), kept.state()), (Honoured, Corrected));
    // The watch next to the encoder never corrects, it only judges.
    let mut watch = HonourMeter::watching();
    assert_eq!(window(&mut watch, budget * 2, TARGET), Corrected);
    assert_eq!(watch.commanded_bps(TARGET), TARGET);
    assert_eq!(window(&mut watch, budget * 5, TARGET), Ignores);
    let mut refused = HonourMeter::watching();
    refused.on_refused();
    assert_eq!(refused.verdict(), Ignores);
    // The guard gives an encoder up after a window of 5× when another one
    // of the codec is left, else keeps it; a refused rate at once; the
    // lossless tiles keep no rate and stay bare.
    let all = HONOUR_WINDOW;
    let (ok, error) = run(&mut watched(5.0, false, CodecKind::H264, true), all);
    assert_eq!(ok, HONOUR_WINDOW - 1);
    assert!(error.unwrap().contains("encoder_ignores_rate"));
    let last = run(&mut watched(5.0, false, CodecKind::H264, false), all);
    assert_eq!(last, (HONOUR_WINDOW, None));
    let tiles = run(&mut watched(5.0, false, CodecKind::Tiles, true), all);
    assert_eq!(tiles, (HONOUR_WINDOW, None));
    let (ok, error) = run(&mut watched(1.0, true, CodecKind::H264, true), 1);
    assert_eq!(ok, 0);
    assert!(error.unwrap().contains("encoder_ignores_rate"));
    // The session's meter: the encoder gets the target times the correction.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_stream_start(20_000_000);
    assert_eq!(rate.encoder_bps(), 20_000_000);
    for _ in 0..HONOUR_WINDOW {
        let given = rate.encoder_bps();
        let bytes = 2 * frame_budget_bytes(given, FPS);
        rate.on_encoded(bytes, false, 20_000_000, given, FPS, "mft");
    }
    assert_eq!(rate.honour(), Corrected);
    assert_eq!(rate.encoder_bps(), 10_000_000);
    assert_eq!(rate.limit(), RateLimit::EncoderIgnores);
    // Another encoder starts over.
    rate.on_encoded(1, true, 20_000_000, 10_000_000, FPS, "nvenc");
    assert_eq!(rate.encoder_bps(), 20_000_000);
    assert_eq!(rate.honour(), Pending);
}

#[test]
fn bildkette_task_rate_mf_h264_buffer_is_bytes() {
    // `CODECAPI_AVEncCommonBufferSize` is bytes for H.264 and HEVC: one
    // frame of 6 Mbit/s at 60 frames/s is 12 500 bytes, not 100 000 bits
    // read as bytes (eight frames, the bursts of the user's session).
    assert_eq!(mf_buffer_bytes(6_000_000, 60), 12_500);
    assert_eq!(mf_buffer_bytes(20_000_000, 60), 41_666);
    assert_eq!(mf_buffer_bytes(6_000_000, 0), 750_000);
    assert_eq!(mf_buffer_bytes(0, 60), 1);
    let budget = frame_budget_bytes(20_000_000, 60);
    assert_eq!(u64::from(mf_buffer_bytes(20_000_000, 60)), budget);
    // Text test, every platform: the MFT gets this value, and reads its
    // mean bit rate back.
    let rate = include_str!("../os/windows/mf_rate.rs").replace("\r\n", "\n");
    assert!(rate.contains("rate_control::model::mf_buffer_bytes(bitrate, fps)"));
    assert!(rate.contains("api.GetValue(key)"));
    assert!(!rate.contains("(bitrate / fps.max(1)).max(1)"));
    let media = include_str!("../os/windows/mf_media.rs");
    assert!(media.contains("set(&CODECAPI_AVEncCommonBufferSize, buffer)"));
    assert!(media.contains("!set_mean_bitrate(&api, bitrate)"));
}

#[test]
fn bildkette_task_rate_encoders_open_at_session_target() {
    // Text test, every platform: the stage opens its encoder at the rate it
    // is given (the target times the correction), no longer at 6 Mbit/s.
    let open = include_str!("codec_registry_open.rs");
    assert!(open.contains("factory.open_admitted_at("));
    assert!(open.contains("request.target_bps,"));
    assert!(open.contains("rate_control::honour::guard("));
    let produce = include_str!("host_pipeline_produce.rs");
    assert!(produce.contains("target_bps: self.rate.encoder_bps(),"));
    assert!(produce.contains("self.rate.set_stream_start(start_table_bps("));
    let mf = include_str!("../os/windows/mf_common.rs");
    assert!(mf.contains("self.create(choice, size, fps, Some(grant), bps)"));
    assert!(mf.contains("transform, setup, grant, bps,"));
    let opened = "let bitrate = bounded_bitrate(bps, u32::MAX);";
    assert!(include_str!("../os/windows/mf_encoder.rs").contains(opened));
    assert!(include_str!("../os/windows/mf_encoder_sync.rs").contains(opened));
    let vendor = include_str!("../os/windows/vendor_codecs.rs");
    assert!(vendor.contains("NvencSession::open_at(&device, codec, size, fps, bps)"));
    assert!(vendor.contains("AmfSession::open_at(&device, codec, size, fps, bps)"));
    let nvenc = include_str!("../os/windows/nvenc_encode.rs");
    let amf = include_str!("../os/windows/amf_encode.rs");
    for text in [nvenc, amf] {
        assert!(text.contains("RateParams::for_target(bps, fps)"));
    }
    // A refused reconfigure keeps the running rate instead of ending NVENC.
    let linux = include_str!("../os/linux_os/nvenc_encode.rs");
    for text in [nvenc, linux] {
        assert!(text.contains("keeps") && text.contains("self.refused"));
        assert!(!text.contains("retune(rate)?;"));
    }
    assert!(amf.contains("self.refused.contains(&self.names.target)"));
}

#[test]
fn bildkette_task_rate_lossless_keyframe_not_stale_on_slower_link() {
    // A 20 MB lossless key frame, target 80 Mbit/s (4K60 table), a line of
    // 30 Mbit/s: it needs 5.3 s. Draining at the target declared it stale
    // after 2.08 s, every time (the reset loop of lossless sessions).
    let mut pacer = Pacer::default();
    pacer.set_timing(20_000, FRAME_US);
    pacer.on_frame_sent(SentFrame {
        gop_id: 1,
        frame_id: 1,
        bytes: 20_000_000,
        sent_us: 0,
        keyframe: true,
    });
    let target = 80_000_000;
    let direct = PathKind::Direct;
    let decide = |pacer: &Pacer| pacer.clone().decide(3_000_000, target, direct, 20_000);
    assert_eq!(pacer.stale_limits(target).0, 2_080_000);
    assert_eq!(decide(&pacer), PaceDecision::ResetStream);
    // At the measured rate it waits until the line could carry it.
    pacer.set_drain_rate(30_000_000);
    assert_eq!(pacer.stale_limits(target).0, 80_000 + 5_333_333);
    assert_ne!(decide(&pacer), PaceDecision::ResetStream);
    // A measured rate above the target drains at the target.
    pacer.set_drain_rate(200_000_000);
    assert_eq!(pacer.stale_limits(target).0, 2_080_000);
    // A stale stream halves the target once per stale time, and the rate
    // the pacer drains at stays the halved one when the application-
    // limited target grows back to its table.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_stream_start(target);
    rate.set_timing(20_000, FRAME_US);
    rate.on_stale_stream(0, 2_080_000);
    assert_eq!(rate.target_bps(), 40_000_000);
    rate.on_stale_stream(500_000, 2_080_000);
    assert_eq!(rate.target_bps(), 40_000_000, "within the stale time");
    for index in 70..140 {
        frame(&mut rate, index, 1_000, 5_000);
    }
    assert_eq!(rate.target_bps(), target);
    assert_eq!(rate.sustained_bps(), 40_000_000);
    rate.on_stale_stream(2_600_000, 2_080_000);
    assert_eq!(rate.target_bps(), 40_000_000);
    // Text test: the pipeline hands the measured rate to the pacer and to
    // the write budget of a frame, and the stale time to the controller.
    let produce = include_str!("host_pipeline_produce.rs");
    assert!(produce.contains("self.pacer.set_drain_rate(self.rate.sustained_bps());"));
    let send = include_str!("host_frame_send.rs");
    assert!(send.contains("u128::from(self.rate.sustained_bps().max(1))"));
    let pipeline = include_str!("host_pipeline.rs");
    assert!(pipeline.contains("self.rate.on_stale_stream(now_us(), stale_us);"));
}
