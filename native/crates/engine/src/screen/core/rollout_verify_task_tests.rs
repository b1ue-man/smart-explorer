//! Measurement of 2026-11-18 (AMD host, AV1): the run-time check failed the
//! encoder because FFmpeg's `av1` decoder, which decodes only with hardware
//! acceleration, gave no picture through `dxva2_vld`. A decoder that gives
//! no picture at all cannot judge the stream; only a decoder that decoded a
//! picture which reads wrong fails the encoder (`Garbled`).
use std::sync::Arc;

use super::fakes::{choice, decoder, encoder, Answer, FakeClock, Fault, SIZE};
use super::{round_trip, Checks};
use crate::screen::core::codec_verdicts::{FailReason, Verdict, VerdictStore};
use crate::screen::core::negotiate::CodecKind;

const H264: CodecKind = CodecKind::H264;

#[test]
fn rollout_task_unverifiable_decoder_does_not_fail_the_encoder() {
    let clock = Arc::new(FakeClock::default());
    let good = encoder("amf-like", H264, true, Fault::None, &clock);
    let pairing = choice(H264, true);
    let trip = |decoders: &[Arc<dyn crate::screen::core::codec_registry::DecoderFactory>]| {
        round_trip(good.as_ref(), decoders, &[], &pairing, SIZE, clock.as_ref())
            .unwrap_or_else(|error| panic!("a fake encoder opens: {error}"))
    };
    // (a) The only decoder gives no picture: not checkable, the encoder is
    // admitted on its stream checks, the reason names the decoder.
    let no_picture = decoder("ffmpeg-dxva2", H264, true, Answer::NoPicture);
    let outcome = trip(&[no_picture.clone()]);
    assert_eq!(outcome.encoder, Verdict::Structural, "{}", outcome.detail);
    assert!(
        outcome.detail.starts_with("not checkable"),
        "{}",
        outcome.detail
    );
    assert!(
        outcome.detail.contains("ffmpeg-dxva2 gave no picture"),
        "{}",
        outcome.detail
    );
    // The decoder is not marked either (it judged nothing).
    assert!(outcome.decoders.is_empty(), "{:?}", outcome.decoders);
    // Another decoder (D3D11VA after DXVA2) that decodes proves the encoder.
    let intact = decoder("ffmpeg-d3d11va", H264, true, Answer::Intact);
    let outcome = trip(&[no_picture.clone(), intact]);
    assert_eq!(outcome.encoder, Verdict::Verified, "{}", outcome.detail);
    // (b) A decoder that decoded a picture which reads wrong still fails it.
    let broken = decoder("broken", H264, false, Answer::Broken);
    let outcome = trip(&[no_picture.clone(), broken]);
    assert_eq!(outcome.encoder, Verdict::Failed(FailReason::Garbled));
    // The gate admits a not checkable encoder.
    let checks = Checks::new(VerdictStore::memory()).with_clock(clock.clone());
    let factory: Arc<dyn crate::screen::core::codec_registry::EncoderFactory> = good.clone();
    assert_eq!(
        checks.admit(&factory, &[no_picture], &pairing, SIZE),
        Ok(())
    );
    // The host log names the result.
    assert!(include_str!("codec_verify_gate.rs").contains("nicht prüfbar (Dekoder: {})"));
}
