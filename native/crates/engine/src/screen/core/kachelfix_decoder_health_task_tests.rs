//! Fix of 2026-11-18 („Kacheln immer“, v0.5.200): a give-up of the decoder
//! guard lasts for its session only, a late picture is a picture, and what a
//! decoder library gave up is kept per viewer session (`for_session`).
use std::time::{Duration, Instant};

use super::{for_session, DecoderGuard, GivenUp, FAILURE_LIMIT, SILENCE_LIMIT};
use crate::screen::core::negotiate::{CodecKind, CodecOffer, H264Profile};

/// A late picture shows that the decoder decodes: it neither counts as a
/// failed key frame nor leaves the silence rule running, so one picture of
/// delay (and a still screen after it) gives nothing up.
#[test]
fn kachelfix_task_late_picture_is_no_failure_and_no_silence() {
    let start = Instant::now();
    let mut guard = DecoderGuard::default();
    assert_eq!(guard.on_record(CodecKind::Hevc, start), None);
    assert_eq!(guard.on_failure(CodecKind::Hevc, start), None);
    assert_eq!(guard.on_picture(CodecKind::Hevc, start, 1), None);
    // Further failures after a picture give nothing up.
    for _ in 0..FAILURE_LIMIT * 2 {
        assert_eq!(guard.on_failure(CodecKind::Hevc, start), None);
    }
    // A still screen: no record, no picture for long.
    assert_eq!(guard.on_tick(start + SILENCE_LIMIT * 10), None);

    // Without any picture the old rules still hold.
    let mut guard = DecoderGuard::default();
    assert_eq!(guard.on_record(CodecKind::H264, start), None);
    assert_eq!(
        guard.on_tick(start + SILENCE_LIMIT - Duration::from_millis(1)),
        None
    );
    assert_eq!(guard.on_tick(start + SILENCE_LIMIT), Some(CodecKind::H264));
    assert!(guard.reason().contains("kein Bild"), "{}", guard.reason());
    let mut guard = DecoderGuard::default();
    for _ in 1..FAILURE_LIMIT {
        assert_eq!(guard.on_failure(CodecKind::Av1, start), None);
    }
    assert_eq!(
        guard.on_failure(CodecKind::Av1, start),
        Some(CodecKind::Av1)
    );
    assert!(
        guard.reason().contains("Schlüsselbilder"),
        "{}",
        guard.reason()
    );
}

/// The codecs one session gave up leave that session's offers only; the
/// tiles are never given up.
#[test]
fn kachelfix_task_given_up_codecs_belong_to_one_session() {
    let offers = [
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::video(CodecKind::Av1, false, true),
        CodecOffer::tiles(),
    ];
    let mut session = GivenUp::default();
    assert_eq!(session.filter(&offers), offers.to_vec());
    assert!(session.insert(CodecKind::Hevc));
    assert!(!session.insert(CodecKind::Hevc), "noted once");
    assert!(!session.insert(CodecKind::Tiles), "the tiles stay");
    assert!(session.contains(CodecKind::Hevc));
    assert!(!session.contains(CodecKind::Tiles));
    assert_eq!(session.keys(), vec!["hevc"]);
    let left: Vec<CodecKind> = session
        .filter(&offers)
        .iter()
        .map(|offer| offer.codec)
        .collect();
    assert_eq!(
        left,
        vec![CodecKind::H264, CodecKind::Av1, CodecKind::Tiles]
    );
    // Another session starts without the give-up.
    let other = GivenUp::default();
    assert_eq!(other.filter(&offers).len(), offers.len());
    for codec in [CodecKind::H264, CodecKind::Av1] {
        assert!(session.insert(codec));
    }
    assert_eq!(session.filter(&offers), vec![CodecOffer::tiles()]);
}

/// What a decoder library gave up is kept for the session that gave it up;
/// a newer session finds the slot empty.
#[test]
fn kachelfix_task_library_give_up_ends_with_its_session() {
    let mut slot: (u64, Vec<CodecKind>) = (0, Vec::new());
    for_session(&mut slot, 3).push(CodecKind::Hevc);
    assert_eq!(*for_session(&mut slot, 3), vec![CodecKind::Hevc]);
    assert!(for_session(&mut slot, 4).is_empty());
    assert_eq!(slot.0, 4);
}
