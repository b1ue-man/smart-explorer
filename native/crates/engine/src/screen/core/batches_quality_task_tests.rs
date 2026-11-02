//! B6 requested quality remains distinct from the temporary effective cadence.
use super::*;

fn policy() -> QualityPolicy {
    QualityPolicy::new(
        CodecChoice {
            codec: CodecKind::H264,
            profile: Some(crate::screen::core::negotiate::H264Profile::High),
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: FrameSize::new(3840, 2160),
            max_pixels: 3840 * 2160,
            max_macroblocks_per_second: None,
        },
        60,
    )
}

#[test]
fn batches_task_b6_full_preview_full_preserves_requested_quality() {
    let mut quality = policy();
    let full = FrameSize::new(3840, 2160);
    let preview = FrameSize::new(640, 360);
    quality.mode = quality
        .mode
        .update(StreamKind::Video, Some(full), Some(48), true, true);
    assert_eq!(quality.mode.size(), Some(full));
    assert_eq!(quality.mode.full_fps, 48);
    assert_eq!(quality.max_fps(), 15);
    quality.mode = quality
        .mode
        .update(StreamKind::Preview, Some(preview), Some(3), true, true);
    assert_eq!(quality.mode.size(), Some(preview));
    assert_eq!(quality.max_fps(), 3);
    quality.mode = quality
        .mode
        .update(StreamKind::Video, None, None, true, true);
    assert_eq!(quality.mode.size(), Some(full));
    assert_eq!((quality.mode.full_fps, quality.mode.preview_fps), (48, 3));
    assert!(quality.mode.sharpness && quality.mode.lossless);
    quality.mode = quality
        .mode
        .update(StreamKind::Video, None, None, false, false);
    assert_eq!(quality.max_fps(), 48);
    assert_eq!(quality.mode.size(), Some(full));
    quality.mode = quality
        .mode
        .update(StreamKind::Preview, None, None, false, false);
    assert_eq!(quality.mode.size(), Some(preview));
    assert_eq!(quality.max_fps(), 3);
}

#[test]
fn batches_task_b6_coalesced_modes_keep_each_requested_rate() {
    let mut quality = policy();
    quality.mode = quality
        .mode
        .update(StreamKind::Video, None, Some(90), false, false)
        .update(StreamKind::Preview, None, Some(2), false, true)
        .update(StreamKind::Video, None, None, false, false);
    assert_eq!(quality.mode.full_fps, 90);
    assert_eq!(quality.mode.preview_fps, 2);
    assert_eq!(quality.max_fps(), 60);
    quality.mode = quality
        .mode
        .update(StreamKind::Video, None, None, true, true);
    assert_eq!(quality.max_fps(), 15);
    assert_eq!(quality.mode.full_fps, 90);
    quality.mode = quality
        .mode
        .update(StreamKind::Video, None, Some(0), false, false);
    assert_eq!(quality.max_fps(), 1);
}
