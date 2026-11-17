//! `DecoderGuard` (fix of 2026-11-17, Windows viewer black): when the
//! negotiated codec brings no picture, the session gives it up once; a codec
//! that showed a picture and the tiles are never given up.
use std::time::{Duration, Instant};

use super::{DecoderGuard, FAILURE_LIMIT, SILENCE_LIMIT};
use crate::screen::core::negotiate::CodecKind;

#[test]
fn bildkette_task_decoder_guard_gives_up_a_codec_without_picture_after_failures_or_silence() {
    let start = Instant::now();
    let mut guard = DecoderGuard::default();
    assert_eq!(guard.on_record(CodecKind::Hevc, start), None);
    for _ in 1..FAILURE_LIMIT {
        assert_eq!(guard.on_failure(CodecKind::Hevc, start), None);
    }
    assert_eq!(
        guard.on_failure(CodecKind::Hevc, start),
        Some(CodecKind::Hevc)
    );
    // Once: the session negotiates again and the guard stays quiet.
    assert_eq!(guard.on_failure(CodecKind::Hevc, start), None);

    // A decoder that fails nothing and delivers nothing.
    let mut guard = DecoderGuard::default();
    assert_eq!(guard.on_record(CodecKind::Av1, start), None);
    let before = start + SILENCE_LIMIT - Duration::from_millis(1);
    assert_eq!(guard.on_tick(before), None);
    assert_eq!(guard.on_tick(start + SILENCE_LIMIT), Some(CodecKind::Av1));

    // The next codec starts its own watch.
    let later = start + SILENCE_LIMIT * 2;
    assert_eq!(guard.on_record(CodecKind::H264, later), None);
    assert_eq!(guard.on_tick(later + Duration::from_secs(1)), None);
}

#[test]
fn bildkette_task_decoder_guard_keeps_a_codec_that_showed_a_picture_and_the_tiles() {
    let start = Instant::now();
    let mut guard = DecoderGuard::default();
    guard.on_record(CodecKind::H264, start);
    guard.on_picture(CodecKind::H264, start);
    for _ in 0..FAILURE_LIMIT * 2 {
        assert_eq!(guard.on_failure(CodecKind::H264, start), None);
    }
    assert_eq!(guard.on_tick(start + SILENCE_LIMIT * 3), None);

    let mut guard = DecoderGuard::default();
    for _ in 0..FAILURE_LIMIT * 2 {
        assert_eq!(guard.on_failure(CodecKind::Tiles, start), None);
    }
    assert_eq!(guard.on_tick(start + SILENCE_LIMIT * 3), None);
}
