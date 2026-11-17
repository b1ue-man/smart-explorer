//! Checking the whole register at once (plan 2026-11-17 „Windows-Bild“
//! P3): `se screen codec verify` (after forgetting the verdicts of this
//! version) and the warm-up of a host (`SessionQuery::VerifyCodecs`, only
//! HEVC and AV1: H.264 is offered unchecked and checked by the gate at its
//! opening). Every video codec and profile of every encoder gets one round
//! trip at `size` (fitted into the encoder's largest picture), unless a
//! verdict of that size class exists.
use serde::{Deserialize, Serialize};

use super::super::codec_verdicts::{Role, Verdict, VerdictView};
use super::super::codec_verify::{Checks, CHECK_FPS};
use super::super::negotiate::{CodecChoice, CodecKind, H264Profile};
use super::super::pixel_convert::FrameSize;
use super::super::synthetic::MIN_SYNTHETIC_SIZE;
use super::CodecRegistry;

/// One encoder, codec and profile of a check of the register.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyLine {
    pub encoder: String,
    pub codec: CodecKind,
    /// The standard's name of the profile (`high`, `cb`, `main`, …).
    pub profile: String,
    pub verdict: VerdictView,
}

/// The view of one verdict; `unchecked` without one.
fn view(verdict: Option<Verdict>) -> VerdictView {
    let (state, reason) = match verdict {
        Some(Verdict::Verified) => ("verified", None),
        Some(Verdict::Structural) => ("structural", None),
        Some(Verdict::Failed(reason)) => ("failed", Some(reason.key().to_string())),
        None => ("unchecked", None),
    };
    VerdictView {
        state: state.into(),
        reason,
    }
}

impl CodecRegistry {
    /// Checks every encoder of a video codec (`newer_only`: HEVC and AV1)
    /// at `size` that has no verdict of its size class yet; one line per
    /// encoder, codec and profile. Nothing without checks.
    pub fn verify_encoders(&self, size: FrameSize, newer_only: bool) -> Vec<VerifyLine> {
        let Some(checks) = self.checks.clone() else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for factory in &self.encoders {
            let mut seen: Vec<(CodecKind, Option<H264Profile>)> = Vec::new();
            for offer in factory.offers() {
                let stream = (offer.codec, offer.profile);
                let skipped = !offer.codec.is_video()
                    || (newer_only && offer.codec == CodecKind::H264)
                    || seen.contains(&stream);
                if skipped {
                    continue;
                }
                seen.push(stream);
                let fitted = size.fit_within(offer.max_size()).even();
                let small = fitted.width < MIN_SYNTHETIC_SIZE.width
                    || fitted.height < MIN_SYNTHETIC_SIZE.height;
                if small || !offer.supports_size_rate(fitted, CHECK_FPS, None) {
                    continue;
                }
                let choice = CodecChoice {
                    codec: offer.codec,
                    profile: offer.profile,
                    encoder_hardware: offer.hardware,
                    decoder_hardware: false,
                    max_size: offer.max_size(),
                    max_pixels: offer.pixel_limit(None),
                    max_macroblocks_per_second: offer.max_macroblocks_per_second,
                };
                let key = Checks::encoder_key(factory.as_ref(), &choice, fitted);
                let verdict = match checks.store().get(&key) {
                    Some(verdict) => Some(verdict),
                    None => checks.check(factory, &self.decoders, &choice, fitted),
                };
                lines.push(VerifyLine {
                    encoder: factory.name(),
                    codec: offer.codec,
                    profile: offer.codec.profile_name(offer.profile).into(),
                    verdict: view(verdict),
                });
            }
        }
        lines
    }

    /// Whether an HEVC or AV1 encoder of this register has no verdict at
    /// all yet (the host then asks for the warm-up).
    pub fn unchecked_newer(&self) -> bool {
        let Some(checks) = &self.checks else {
            return false;
        };
        self.encoders.iter().any(|factory| {
            let (name, device) = (factory.name(), factory.device());
            factory
                .offers()
                .iter()
                .filter(|offer| matches!(offer.codec, CodecKind::Hevc | CodecKind::Av1))
                .any(|offer| {
                    let profile = Some(offer.codec.profile_name(offer.profile));
                    !checks
                        .store()
                        .summary(&name, &device, offer.codec, profile, Role::Encoder)
                        .checked()
                })
        })
    }
}
