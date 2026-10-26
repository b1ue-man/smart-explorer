//! B6 atomic hybrid transactions at the actual ScreenDecoder boundary.
use super::*;
use crate::screen::core::frame_budget::{FrameLimits, FRAME_PROTOCOL};
use crate::screen::core::frame_regions::{grid, MoveRect};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::FLAG_MORE;

const SIZE: FrameSize = FrameSize::new(128, 64);

#[derive(Clone, Copy)]
enum VideoOutput {
    Current,
    Delayed,
    Missing,
    NeedsInput,
}

struct VideoFixture {
    image: BgraImage,
    output: VideoOutput,
    identity: Option<DecodedFrame>,
}
impl ScreenDecoder for VideoFixture {
    fn name(&self) -> String {
        "b6-decoder-output-fixture".into()
    }
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.identity = match self.output {
            VideoOutput::Current => Some(DecodedFrame::from_record(record)),
            VideoOutput::Delayed => Some(DecodedFrame {
                frame_id: record.frame_id.saturating_sub(1),
                capture_ts_us: record.capture_ts_us.saturating_sub(1),
                keyframe: record.is_keyframe(),
            }),
            VideoOutput::Missing | VideoOutput::NeedsInput => None,
        };
        Ok(!matches!(self.output, VideoOutput::NeedsInput))
    }
    fn picture(&self) -> &BgraImage {
        &self.image
    }
    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.identity
    }
}

fn image(seed: u8) -> BgraImage {
    let mut image = BgraImage::new(SIZE).unwrap();
    for (i, pixel) in image.data_mut().chunks_exact_mut(4).enumerate() {
        pixel.copy_from_slice(&[seed.wrapping_add(i as u8), (i / 128) as u8, seed, 255]);
    }
    image
}
fn decoder(output: VideoOutput) -> HybridDecoder {
    let contract = FrameContract {
        version: FRAME_PROTOCOL,
        limits: FrameLimits {
            max_size: SIZE,
            max_pixels: SIZE.pixels(),
            working_bytes: 4 * 1024 * 1024,
        },
        hybrid_regions: true,
    };
    HybridDecoder::new(
        Box::new(VideoFixture {
            image: image(70),
            output,
            identity: None,
        }),
        SIZE,
        contract,
    )
    .unwrap()
}
fn stamp(frame_id: u64, base_frame_id: u64) -> FrameStamp {
    FrameStamp {
        generation: 3,
        frame_id,
        base_frame_id,
    }
}
fn record(stamp: FrameStamp, op: Operation, key: bool) -> FrameRecord {
    let more = !matches!(&op, Operation::Commit);
    FrameRecord {
        frame_id: stamp.frame_id,
        capture_ts_us: stamp.frame_id * 100,
        flags: if key { FLAG_KEYFRAME } else { 0 } | if more { FLAG_MORE } else { 0 },
        payload: frame_protocol::encode(stamp, op).unwrap(),
    }
}
fn seed(decoder: &mut HybridDecoder) -> BgraImage {
    let expected = image(10);
    let id = stamp(1, 0);
    assert!(!decoder.decode(&record(id, Operation::Begin, true)).unwrap());
    for rect in grid(SIZE) {
        assert!(!decoder
            .decode(&record(
                id,
                frame_protocol::lossless(&expected, rect).unwrap(),
                true
            ))
            .unwrap());
    }
    assert!(decoder
        .decode(&record(id, Operation::Commit, true))
        .unwrap());
    assert_eq!(decoder.picture().data(), expected.data());
    expected
}
fn assert_unpublished(decoder: &HybridDecoder, before: &BgraImage) {
    assert_eq!(decoder.picture().data(), before.data());
    assert_eq!(decoder.decoded_frame(), None);
}
fn video() -> Operation {
    Operation::Video {
        keyframe: true,
        rects: vec![grid(SIZE)[0]],
        data: vec![1],
    }
}

#[test]
fn batches_task_b6_video_moves_dirty_publish_only_on_commit() {
    let mut decoder = decoder(VideoOutput::Current);
    let before = seed(&mut decoder);
    let id = stamp(2, 1);
    let rects = grid(SIZE);
    let correction = image(120);
    let dirty = Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 8,
    };
    let operations = [
        Operation::Begin,
        video(),
        Operation::Moves(vec![MoveRect {
            source_x: 0,
            source_y: 0,
            destination: rects[1],
        }]),
        frame_protocol::lossless(&correction, dirty).unwrap(),
    ];
    for op in operations {
        assert!(!decoder.decode(&record(id, op, false)).unwrap());
        assert_unpublished(&decoder, &before);
    }
    let commit = record(id, Operation::Commit, false);
    assert!(decoder.decode(&commit).unwrap());
    assert_eq!(
        decoder.decoded_frame(),
        Some(DecodedFrame::from_record(&commit))
    );
    let video_image = image(70);
    for y in 0..SIZE.height {
        for x in 0..SIZE.width {
            let (source, source_x) = if x < 8 && y < 8 {
                (&correction, x)
            } else if x < 64 {
                (&video_image, x)
            } else {
                (&before, x - 64)
            };
            assert_eq!(
                &decoder.picture().view().row(y)[x as usize * 4..x as usize * 4 + 4],
                &source.view().row(y)[source_x as usize * 4..source_x as usize * 4 + 4]
            );
        }
    }
}

#[test]
fn batches_task_b6_lossless_correction_matches_every_source_pixel() {
    let mut decoder = decoder(VideoOutput::Current);
    let before = seed(&mut decoder);
    let expected = image(211);
    let id = stamp(2, 1);
    assert!(!decoder
        .decode(&record(id, Operation::Begin, false))
        .unwrap());
    for rect in grid(SIZE) {
        assert!(!decoder
            .decode(&record(
                id,
                frame_protocol::lossless(&expected, rect).unwrap(),
                false
            ))
            .unwrap());
        assert_unpublished(&decoder, &before);
    }
    assert!(decoder
        .decode(&record(id, Operation::Commit, false))
        .unwrap());
    assert_eq!(decoder.picture().size(), SIZE);
    assert_eq!(decoder.picture().data(), expected.data());
    let quality =
        crate::screen::core::quality_acceptance_metrics::measure(&expected, decoder.picture())
            .unwrap();
    assert_eq!(quality.different_pixels, 0);
    assert_eq!(quality.ssim_luma, 1.0);
    assert!(quality.psnr_infinite);
    crate::screen::core::quality_acceptance_metrics::evidence(
        "deterministic-lossless-still",
        serde_json::json!({
        "capture_sha256":crate::screen::core::quality_acceptance_metrics::sha256(expected.data()),
        "quality":quality,"physical_codec":false,"result":"passed"}),
    );
}

#[test]
fn batches_task_b6_quality_reference_loss_recovers_only_with_complete_keyframe() {
    use crate::screen::core::quality_acceptance_metrics as metrics;
    let mut decoder = decoder(VideoOutput::Current);
    let before = seed(&mut decoder);
    let expected = image(211);
    let incomplete = stamp(2, 0);
    assert!(!decoder
        .decode(&record(incomplete, Operation::Begin, true))
        .unwrap());
    assert!(!decoder
        .decode(&record(
            incomplete,
            frame_protocol::lossless(&expected, grid(SIZE)[0]).unwrap(),
            true
        ))
        .unwrap());
    assert!(decoder
        .decode(&record(incomplete, Operation::Commit, true))
        .is_err());
    assert_unpublished(&decoder, &before);
    assert!(decoder
        .decode(&record(stamp(3, 1), Operation::Begin, false))
        .is_err());
    assert_unpublished(&decoder, &before);
    let held = metrics::measure(&expected, decoder.picture()).unwrap();
    assert!(held.different_pixels > 0);
    let recovery = stamp(4, 0);
    assert!(!decoder
        .decode(&record(recovery, Operation::Begin, true))
        .unwrap());
    for rect in grid(SIZE) {
        assert!(!decoder
            .decode(&record(
                recovery,
                frame_protocol::lossless(&expected, rect).unwrap(),
                true
            ))
            .unwrap());
        assert_unpublished(&decoder, &before);
    }
    let commit = record(recovery, Operation::Commit, true);
    assert!(decoder.decode(&commit).unwrap());
    assert_eq!(
        decoder.decoded_frame(),
        Some(DecodedFrame::from_record(&commit))
    );
    let recovered = metrics::measure(&expected, decoder.picture()).unwrap();
    assert_eq!(recovered.different_pixels, 0);
    assert_eq!(recovered.ssim_luma, 1.0);
    assert!(recovered.psnr_infinite);
    metrics::evidence(
        "deterministic-reference-recovery",
        serde_json::json!({
        "capture_sha256":metrics::sha256(expected.data()),
        "withheld_frame_ids":[2,3],"accepted_recovery_frame_id":4,
        "capture_to_held_last_valid":held,"capture_to_first_complete_keyframe":recovered,
        "missing_output_quality":null,"physical_codec":false,
        "scope":"real hybrid transaction/reference gate with a replacement video decoder; no physical codec-quality pass",
        "result":"passed"}),
    );
}

#[test]
fn batches_task_b6_missing_or_foreign_base_cannot_publish() {
    let mut fresh = decoder(VideoOutput::Current);
    let black = fresh.picture().clone();
    assert!(fresh
        .decode(&record(stamp(2, 1), Operation::Begin, false))
        .is_err());
    assert_unpublished(&fresh, &black);
    for id in [
        stamp(2, 0),
        stamp(2, 9),
        FrameStamp {
            generation: 4,
            ..stamp(2, 1)
        },
        stamp(1, 1),
    ] {
        let mut decoder = decoder(VideoOutput::Current);
        let before = seed(&mut decoder);
        assert!(decoder
            .decode(&record(id, Operation::Begin, false))
            .is_err());
        assert_unpublished(&decoder, &before);
    }
}

#[test]
fn batches_task_b6_mixed_frame_time_generation_or_flags_abort_transaction() {
    for mismatch in 0..6 {
        let mut decoder = decoder(VideoOutput::Current);
        let before = seed(&mut decoder);
        let id = stamp(2, 1);
        assert!(!decoder
            .decode(&record(id, Operation::Begin, false))
            .unwrap());
        let mut op_id = id;
        match mismatch {
            0 => op_id.frame_id += 1,
            1 => op_id.generation += 1,
            2 => op_id.base_frame_id += 1,
            _ => {}
        }
        let mut dirty = record(
            op_id,
            frame_protocol::lossless(&image(90), grid(SIZE)[0]).unwrap(),
            false,
        );
        match mismatch {
            3 => dirty.capture_ts_us += 1,
            4 => dirty.frame_id += 1,
            5 => dirty.flags |= FLAG_KEYFRAME,
            _ => {}
        }
        assert!(decoder.decode(&dirty).is_err(), "mismatch {mismatch}");
        assert_unpublished(&decoder, &before);
        assert!(decoder
            .decode(&record(id, Operation::Commit, false))
            .is_err());
        assert_unpublished(&decoder, &before);
    }
}

#[test]
fn batches_task_b6_incomplete_keyframe_and_wrong_order_keep_old_pixels() {
    let mut decoder = decoder(VideoOutput::Current);
    let before = seed(&mut decoder);
    let key = stamp(2, 0);
    assert!(!decoder
        .decode(&record(key, Operation::Begin, true))
        .unwrap());
    assert!(!decoder
        .decode(&record(
            key,
            frame_protocol::lossless(&image(90), grid(SIZE)[0]).unwrap(),
            true
        ))
        .unwrap());
    assert!(decoder
        .decode(&record(key, Operation::Commit, true))
        .is_err());
    assert_unpublished(&decoder, &before);
    // A failure invalidates the reference: only a new complete keyframe recovers.
    assert!(decoder
        .decode(&record(stamp(3, 1), Operation::Begin, false))
        .is_err());
    let key = stamp(4, 0);
    assert!(!decoder
        .decode(&record(key, Operation::Begin, true))
        .unwrap());
    let recovered = image(44);
    for rect in grid(SIZE) {
        assert!(!decoder
            .decode(&record(
                key,
                frame_protocol::lossless(&recovered, rect).unwrap(),
                true
            ))
            .unwrap());
    }
    assert!(decoder
        .decode(&record(key, Operation::Commit, true))
        .unwrap());
    let next = stamp(5, 4);
    assert!(!decoder
        .decode(&record(next, Operation::Begin, false))
        .unwrap());
    assert!(!decoder
        .decode(&record(
            next,
            frame_protocol::lossless(&image(91), grid(SIZE)[0]).unwrap(),
            false
        ))
        .unwrap());
    assert!(decoder
        .decode(&record(next, Operation::Moves(Vec::new()), false))
        .is_err());
    assert_unpublished(&decoder, &recovered);
}

#[test]
fn batches_task_b6_delayed_or_unidentified_video_never_composites() {
    for output in [
        VideoOutput::Delayed,
        VideoOutput::Missing,
        VideoOutput::NeedsInput,
    ] {
        let mut decoder = decoder(output);
        let before = seed(&mut decoder);
        let id = stamp(2, 1);
        assert!(!decoder
            .decode(&record(id, Operation::Begin, false))
            .unwrap());
        let error = decoder.decode(&record(id, video(), false)).unwrap_err();
        let expected = if matches!(output, VideoOutput::NeedsInput) {
            DecoderFallback::NeedsMoreInput
        } else {
            DecoderFallback::OutputIdentityUnavailable
        };
        assert_eq!(DecoderFallback::from_error(&error), Some(expected));
        assert_unpublished(&decoder, &before);
        assert!(decoder
            .decode(&record(id, Operation::Commit, false))
            .is_err());
        // The native decoder may still retain the failed AU; reusing its id is forbidden.
        let reused = stamp(2, 0);
        assert!(!decoder
            .decode(&record(reused, Operation::Begin, true))
            .unwrap());
        assert!(decoder.decode(&record(reused, video(), true)).is_err());
        assert_unpublished(&decoder, &before);
    }
}
