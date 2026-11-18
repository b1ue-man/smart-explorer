//! Task tests of the run-time codec check (plan 2026-11-17 „Windows-Bild“
//! P3, TODO `VERB-WIN-KODIERER`) without a GPU: fake „H.264“ and „HEVC“
//! coders around the lossless tile codec (`bildkette_verify_fakes.rs`) run
//! the real round trip, gate, verdict store and negotiation.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::fakes::{choice, decoder, encoder, Answer, FakeClock, Fault, SIZE};
use super::{round_trip, Checks, Outcome, VerifyClock};
use crate::screen::core::codec_registry::{CodecRegistry, DecoderFactory, EncoderFactory};
use crate::screen::core::codec_verdicts::{
    FailReason, Role, Verdict, VerdictKey, VerdictStore, VerdictView, CRASH_AFTER_SECS,
    PROGRAM_VERSION,
};
use crate::screen::core::negotiate::{
    negotiate_trusted, CodecKind, CodecOffer, CodecPreference, CodecTrust, H264Profile,
};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::set_decoders::renegotiate_trusted;

const H264: CodecKind = CodecKind::H264;
const HEVC: CodecKind = CodecKind::Hevc;

fn run(
    encoder: &dyn EncoderFactory,
    decoders: &[Arc<dyn DecoderFactory>],
    codec: CodecKind,
    clock: &FakeClock,
) -> Outcome {
    round_trip(encoder, decoders, &[], &choice(codec, true), SIZE, clock)
        .expect("a fake encoder opens")
}

fn checks(clock: &Arc<FakeClock>) -> Arc<Checks> {
    Arc::new(Checks::new(VerdictStore::memory()).with_clock(clock.clone()))
}

#[test]
fn bildkette_task_verify_tiles_round_trip_verified() {
    let clock = Arc::new(FakeClock::default());
    let good = encoder("fake-h264", H264, true, Fault::None, &clock);
    let decoders = vec![decoder("fake-decoder", H264, false, Answer::Intact)];
    let outcome = run(good.as_ref(), &decoders, H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Verified, "{}", outcome.detail);
    assert_eq!(outcome.decoders, vec![(0, Verdict::Verified)]);
    // Pictures 3 to 6 follow at 30 per second.
    assert!(
        clock.now() >= Duration::from_micros(99_999),
        "{:?}",
        clock.now()
    );

    // The gate checks once and keeps the verdict under its key.
    let checks = checks(&clock);
    let factory: Arc<dyn EncoderFactory> = good.clone();
    let pairing = choice(H264, true);
    assert_eq!(checks.admit(&factory, &decoders, &pairing, SIZE), Ok(()));
    let opened = good.opened.load(Ordering::SeqCst);
    assert_eq!(checks.admit(&factory, &decoders, &pairing, SIZE), Ok(()));
    assert_eq!(
        good.opened.load(Ordering::SeqCst),
        opened,
        "no second round trip"
    );
    let key = Checks::encoder_key(good.as_ref(), &pairing, SIZE);
    assert_eq!(checks.store().get(&key), Some(Verdict::Verified));
    assert_eq!(
        (
            key.version.as_str(),
            key.profile.as_str(),
            key.size_class.as_str()
        ),
        (PROGRAM_VERSION, "high", "720p")
    );
    assert_eq!(key.device, "10de:0001:a1@1.2.3.4");
    let decoder_key = Checks::decoder_key(decoders[0].as_ref(), H264, SIZE);
    assert_eq!(checks.store().get(&decoder_key), Some(Verdict::Verified));
}

#[test]
fn bildkette_task_verify_garbage_encoder_failed_garbled() {
    let clock = Arc::new(FakeClock::default());
    let garbage = encoder("garbage", H264, true, Fault::Garbage, &clock);
    let decoders = vec![decoder("fake-decoder", H264, false, Answer::Intact)];
    let outcome = run(garbage.as_ref(), &decoders, H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Failed(FailReason::Garbled));
    // The reference decoder is not blamed for the encoder's pictures.
    assert!(outcome.decoders.is_empty(), "{outcome:?}");
}

#[test]
fn bildkette_task_verify_slow_encoder_failed_slow() {
    let clock = Arc::new(FakeClock::default());
    let slow = encoder("slow", H264, true, Fault::Slow, &clock);
    let decoders = vec![decoder("fake-decoder", H264, false, Answer::Intact)];
    let outcome = run(slow.as_ref(), &decoders, H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Failed(FailReason::Slow));
    assert!(outcome.detail.contains("250"), "{}", outcome.detail);
}

#[test]
fn bildkette_task_verify_late_encoder_and_late_decoder_attributed() {
    let clock = Arc::new(FakeClock::default());
    let intact = decoder("intact", H264, true, Answer::Intact);
    let late = decoder("late", H264, false, Answer::Late);
    let broken = decoder("broken", H264, false, Answer::Broken);
    // An encoder whose first picture comes out of its second call.
    let lagging = encoder("lagging", H264, true, Fault::Late, &clock);
    let outcome = run(lagging.as_ref(), &[intact.clone()], H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Failed(FailReason::Late));
    assert!(outcome.decoders.is_empty());
    let good = encoder("good", H264, true, Fault::None, &clock);
    // A late decoder (software, tried first) is blamed; the next decoder
    // proves the encoder.
    let outcome = run(good.as_ref(), &[late.clone(), intact.clone()], H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Verified);
    assert_eq!(
        outcome.decoders,
        vec![
            (0, Verdict::Failed(FailReason::Late)),
            (1, Verdict::Verified)
        ]
    );
    // Only a late decoder: the stream is checked, the pixels are not.
    let outcome = run(good.as_ref(), &[late], H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Structural);
    assert_eq!(
        outcome.decoders,
        vec![(0, Verdict::Failed(FailReason::Late))]
    );
    // A broken decoder is blamed once another one decodes the stream intact.
    let outcome = run(good.as_ref(), &[broken.clone(), intact], H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Verified);
    assert_eq!(
        outcome.decoders,
        vec![
            (1, Verdict::Verified),
            (0, Verdict::Failed(FailReason::Garbled))
        ]
    );
    // Alone it blames the encoder.
    let outcome = run(good.as_ref(), &[broken], H264, &clock);
    assert_eq!(outcome.encoder, Verdict::Failed(FailReason::Garbled));
}

#[test]
fn bildkette_task_verify_missing_keyframe_failed() {
    let clock = Arc::new(FakeClock::default());
    let deaf = encoder("deaf", H264, true, Fault::IgnoresKeyframe, &clock);
    let decoders = vec![decoder("fake-decoder", H264, false, Answer::Intact)];
    let outcome = run(deaf.as_ref(), &decoders, H264, &clock);
    assert_eq!(
        outcome.encoder,
        Verdict::Failed(FailReason::MissingKeyframe)
    );
    assert!(outcome.detail.contains("picture 5"), "{}", outcome.detail);
}

#[test]
fn bildkette_task_verify_no_reference_decoder_structural_or_missing_parameter_sets() {
    let clock = Arc::new(FakeClock::default());
    // HEVC without an HEVC decoder here (no Store extension): the stream and
    // the time are checked.
    let hevc = encoder("hevc", HEVC, true, Fault::None, &clock);
    let only_h264 = vec![decoder("h264-only", H264, false, Answer::Intact)];
    let outcome = run(hevc.as_ref(), &only_h264, HEVC, &clock);
    assert_eq!(outcome.encoder, Verdict::Structural, "{}", outcome.detail);
    assert!(outcome.decoders.is_empty());
    // A first key frame without VPS, SPS and PPS fails without a decoder.
    let bare = encoder("bare", HEVC, true, Fault::NoParameterSets, &clock);
    let outcome = run(bare.as_ref(), &[], HEVC, &clock);
    assert_eq!(
        outcome.encoder,
        Verdict::Failed(FailReason::MissingParameterSets)
    );
    // With an HEVC decoder it is verified.
    let with_decoder = vec![decoder("hevc-decoder", HEVC, false, Answer::Intact)];
    let outcome = run(hevc.as_ref(), &with_decoder, HEVC, &clock);
    assert_eq!(outcome.encoder, Verdict::Verified);
}

#[test]
fn bildkette_task_verify_open_skips_failed_factory_and_offers_drop_it() {
    let clock = Arc::new(FakeClock::default());
    let bad = encoder("bad-hw", H264, true, Fault::Garbage, &clock);
    let good = encoder("good-sw", H264, false, Fault::None, &clock);
    let hevc = encoder("hevc-hw", HEVC, true, Fault::None, &clock);
    let h264_decoder = decoder("h264-decoder", H264, false, Answer::Intact);
    let checks = checks(&clock);
    let registry = CodecRegistry::new(
        vec![
            bad.clone() as Arc<dyn EncoderFactory>,
            good.clone() as Arc<dyn EncoderFactory>,
            hevc.clone() as Arc<dyn EncoderFactory>,
        ],
        vec![h264_decoder.clone()],
    )
    .with_checks(checks.clone());
    let high = |hardware| CodecOffer::h264(H264Profile::High, hardware).with_max(FrameSize::MAX);
    // Unchecked: H.264 offered (the gate protects it), HEVC not.
    let offers = registry.encoder_offers();
    assert!(offers.contains(&high(true)) && offers.contains(&high(false)));
    assert!(offers.iter().all(|offer| offer.codec != HEVC), "{offers:?}");
    // The hardware encoder ranks first, fails its check and is skipped.
    let opened = registry
        .open_encoder_with_contract(&choice(H264, true), SIZE, 30, &[], None)
        .unwrap();
    assert_eq!(opened.factory, "good-sw");
    let offers = registry.encoder_offers();
    assert!(!offers.contains(&high(true)) && offers.contains(&high(false)));
    // A failed encoder is skipped from its verdict, without a new check.
    let bad_opened = bad.opened.load(Ordering::SeqCst);
    let again = registry
        .open_encoder_with_contract(&choice(H264, true), SIZE, 30, &[], None)
        .unwrap();
    assert_eq!(again.factory, "good-sw");
    assert_eq!(bad.opened.load(Ordering::SeqCst), bad_opened);
    // The warm-up checks HEVC; then it is offered.
    let lines = registry.verify_encoders(SIZE, true);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].verdict.state, "structural");
    assert!(registry
        .encoder_offers()
        .iter()
        .any(|offer| offer.codec == HEVC));
    assert!(!registry.unchecked_newer());
    // The report names every verdict; the tiles have none.
    let report = registry.report(crate::screen::core::codec_registry::OpenH264Report::new(
        crate::screen::core::codec_registry::OpenH264State::Absent,
        None,
    ));
    let bad_row = report
        .encoders
        .iter()
        .find(|row| row.name == "bad-hw")
        .unwrap();
    assert_eq!(
        bad_row.verdict,
        Some(VerdictView {
            state: "failed".into(),
            reason: Some("garbled".into())
        })
    );
    assert_eq!(report.encoders.last().unwrap().verdict, None);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["encoders"][0]["verdict"]["state"], "failed");
    // A decoder that failed its check is not offered while another decoder
    // of its codec remains. Fix of 2026-11-18 („Kacheln immer“): the sole
    // decoder of a codec stays offered (its verdict judged another stream;
    // the session's guard judges the real one) instead of the codec
    // silently leaving the hello.
    assert!(registry
        .decoder_offers()
        .iter()
        .any(|offer| offer.codec == H264));
    let key = Checks::decoder_key(h264_decoder.as_ref(), H264, SIZE);
    checks
        .store()
        .record(&key, Verdict::Failed(FailReason::Late), "test");
    assert!(
        registry
            .decoder_offers()
            .iter()
            .any(|offer| offer.codec == H264),
        "the sole H.264 decoder stays offered"
    );
    let hardware = decoder("h264-hw", H264, true, Answer::Intact);
    let both = CodecRegistry::new(Vec::new(), vec![h264_decoder.clone(), hardware])
        .with_checks(checks.clone());
    let offers = both.decoder_offers();
    assert!(
        offers
            .iter()
            .any(|offer| offer.codec == H264 && offer.hardware),
        "{offers:?}"
    );
    assert!(
        offers
            .iter()
            .all(|offer| offer.codec != H264 || offer.hardware),
        "the failed decoder leaves while another remains: {offers:?}"
    );
}

#[test]
fn bildkette_task_verify_verdict_store_key_change_and_crash_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("screen").join("codec_verdicts.json");
    let now = Arc::new(AtomicU64::new(1_000_000));
    let time = now.clone();
    let store = VerdictStore::at(path.clone()).with_clock(move || time.load(Ordering::SeqCst));
    let device = "10de:0001:a1@1.2.3.4";
    let key = VerdictKey::new("nvenc-hevc", device, HEVC, None, SIZE, Role::Encoder);
    store.record(&key, Verdict::Verified, "intact");
    // Another process reads the file.
    assert_eq!(
        VerdictStore::at(path.clone()).get(&key),
        Some(Verdict::Verified)
    );
    // A new driver, program version or size class checks again.
    let driver = VerdictKey {
        device: "10de:0001:a1@1.2.3.5".into(),
        ..key.clone()
    };
    let version = VerdictKey {
        version: "0.0.1".into(),
        ..key.clone()
    };
    let large = VerdictKey::new(
        "nvenc-hevc",
        device,
        HEVC,
        None,
        FrameSize::new(3840, 2160),
        Role::Encoder,
    );
    assert_eq!(large.size_class, "2160p");
    for other in [&driver, &version, &large] {
        assert_eq!(store.get(other), None, "{other:?}");
    }
    // A process that ends inside its check leaves the running mark; the
    // second such attempt is a crash, a third is not started.
    let crash = VerdictKey::new("amf-hevc", "1002:0002:c1", HEVC, None, SIZE, Role::Encoder);
    assert!(store.begin(&crash));
    assert_eq!(store.get(&crash), None, "a check runs");
    now.fetch_add(CRASH_AFTER_SECS + 1, Ordering::SeqCst);
    assert_eq!(store.get(&crash), None, "one attempt is no crash yet");
    assert!(store.begin(&crash));
    now.fetch_add(CRASH_AFTER_SECS + 1, Ordering::SeqCst);
    assert_eq!(
        store.get(&crash),
        Some(Verdict::Failed(FailReason::Crashed))
    );
    assert!(!store.begin(&crash));
    assert_eq!(
        VerdictStore::at(path.clone()).get(&crash),
        Some(Verdict::Failed(FailReason::Crashed))
    );
    // A check that could not run leaves nothing.
    let cancelled = VerdictKey::new("mft", "", HEVC, None, SIZE, Role::Encoder);
    assert!(store.begin(&cancelled));
    store.cancel(&cancelled);
    assert_eq!(store.get(&cancelled), None);
    // `se screen codec verify` forgets the verdicts of this version.
    store.forget();
    assert_eq!(store.get(&key), None);
    assert_eq!(VerdictStore::at(path).get(&crash), None);
}

/// The hardware pairings of H.264 and HEVC (and AV1).
fn hardware_offers(codecs: &[CodecKind]) -> Vec<CodecOffer> {
    let mut offers: Vec<CodecOffer> = codecs
        .iter()
        .map(|codec| match codec {
            CodecKind::H264 => CodecOffer::h264(H264Profile::High, true),
            codec => CodecOffer::video(*codec, false, true),
        })
        .collect();
    offers.push(CodecOffer::tiles());
    offers
}

#[test]
fn bildkette_task_verify_negotiate_unverified_hevc_after_verified_h264() {
    let clock = Arc::new(FakeClock::default());
    let registry = CodecRegistry::new(
        vec![
            encoder("h264", H264, true, Fault::None, &clock) as Arc<dyn EncoderFactory>,
            encoder("hevc", HEVC, true, Fault::None, &clock) as Arc<dyn EncoderFactory>,
        ],
        vec![decoder("h264-decoder", H264, false, Answer::Intact)],
    )
    .with_checks(checks(&clock));
    let lines = registry.verify_encoders(SIZE, false);
    let states: Vec<&str> = lines
        .iter()
        .map(|line| line.verdict.state.as_str())
        .collect();
    assert_eq!(states, ["verified", "structural"]);
    // HEVC is offered (structural) but keeps no rank before H.264 (D6).
    assert!(registry
        .encoder_offers()
        .iter()
        .any(|offer| offer.codec == HEVC));
    let trust = registry.codec_trust();
    assert!(trust.trusts(H264) && !trust.trusts(HEVC));
    let offers = hardware_offers(&[H264, HEVC]);
    let choices = negotiate_trusted(&offers, &offers, CodecPreference::Auto, None, trust);
    let order: Vec<CodecKind> = choices.iter().map(|choice| choice.codec).collect();
    assert_eq!(order, [H264, HEVC, CodecKind::Tiles]);
    let renegotiated = renegotiate_trusted(&offers, &offers, None, true, trust).unwrap();
    assert_eq!(renegotiated.choice.codec, H264);
    assert_eq!(renegotiated.fallbacks[0].codec, HEVC);
}

#[test]
fn bildkette_task_verify_negotiate_verified_newer_codec_keeps_b30() {
    let clock = Arc::new(FakeClock::default());
    let registry = CodecRegistry::new(
        vec![
            encoder("h264", H264, true, Fault::None, &clock) as Arc<dyn EncoderFactory>,
            encoder("hevc", HEVC, true, Fault::None, &clock) as Arc<dyn EncoderFactory>,
        ],
        vec![
            decoder("h264-decoder", H264, false, Answer::Intact),
            decoder("hevc-decoder", HEVC, false, Answer::Intact),
        ],
    )
    .with_checks(checks(&clock));
    registry.verify_encoders(SIZE, false);
    let trust = registry.codec_trust();
    assert!(trust.trusts(HEVC) && !trust.trusts(CodecKind::Av1));
    let offers = hardware_offers(&[H264, HEVC, CodecKind::Av1]);
    let order = |trust| -> Vec<CodecKind> {
        negotiate_trusted(&offers, &offers, CodecPreference::Auto, None, trust)
            .iter()
            .map(|choice| choice.codec)
            .collect()
    };
    // Verified HEVC keeps its B-30 rank; unchecked AV1 follows H.264.
    assert_eq!(order(trust), [HEVC, H264, CodecKind::Av1, CodecKind::Tiles]);
    // A host that checks nothing: HEVC, H.264, and AV1 last (P9: AV1
    // never ranks first in automatic).
    assert_eq!(
        order(CodecTrust::ALL),
        [HEVC, H264, CodecKind::Av1, CodecKind::Tiles]
    );
    assert_eq!(
        CodecRegistry::new(Vec::new(), Vec::new()).codec_trust(),
        CodecTrust::ALL
    );
    // Outside the hardware/hardware tier H.264 comes first either way.
    let software = [
        CodecOffer::h264(H264Profile::High, false),
        CodecOffer::video(HEVC, false, false),
        CodecOffer::tiles(),
    ];
    let first = negotiate_trusted(&software, &software, CodecPreference::Auto, None, trust)[0];
    assert_eq!(first.codec, H264);
}
