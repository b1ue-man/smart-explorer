//! What the viewer's register says before a decoder exists (plan
//! 2026-11-14 „Bildkette“ B-36 and V1): whether the viewer may announce
//! `gop_continue` for its offers, and whether a new „Dekodierer“ choice can
//! take over a running stream without a new session.
use super::super::frame_budget::FrameContract;
use super::super::negotiate::{CodecKind, CodecOffer, DecoderPreference, H264Profile};
use super::super::pixel_convert::FrameSize;
use super::CodecRegistry;

impl CodecRegistry {
    /// B-36: `true` when every decoder this register may open for a video
    /// codec of `offers` under `preference` keeps its references over a
    /// continuation (`DecoderFactory::keeps_references`). A video codec
    /// that some admitted decoder cannot continue (it may be the one that
    /// opens, or the fallback after a failure) and offers without any video
    /// codec make it `false`: the viewer then does not announce the
    /// capability and the host resets with a key frame, as before.
    pub fn keeps_references(&self, offers: &[CodecOffer], preference: DecoderPreference) -> bool {
        let mut video = offers
            .iter()
            .filter(|offer| offer.codec.is_video())
            .peekable();
        if video.peek().is_none() {
            return false;
        }
        video.all(|offer| self.offer_continues(offer, preference))
    }

    /// B-36 per codec (P1): the video offers of `offers` for which every
    /// decoder this register may open under `preference` keeps its
    /// references (`gop_continue_decoders`). Under „Automatisch“ with H.264,
    /// HEVC and AV1 on Windows these are the HEVC and AV1 offers.
    pub fn continuing_offers(
        &self,
        offers: &[CodecOffer],
        preference: DecoderPreference,
    ) -> Vec<CodecOffer> {
        offers
            .iter()
            .filter(|offer| offer.codec.is_video() && self.offer_continues(offer, preference))
            .copied()
            .collect()
    }

    /// Every decoder that may open for `offer` continues, and one exists.
    fn offer_continues(&self, offer: &CodecOffer, preference: DecoderPreference) -> bool {
        let mut admitted = self
            .decoders
            .iter()
            .filter(|factory| {
                factory.offers().iter().any(|own| {
                    own.decodes(offer.codec, offer.profile) && preference.allows(own.hardware)
                })
            })
            .peekable();
        admitted.peek().is_some() && admitted.all(|factory| factory.keeps_references(offer.codec))
    }

    /// V1: whether a decoder of `preference`'s stage opens for a running
    /// stream of `codec`/`profile` at `size`, the same test as
    /// `open_decoder_raw`. Then the viewer changes its decoder at the next
    /// key frame it asks for; otherwise it opens the session again.
    pub fn decodes_with(
        &self,
        stream: (CodecKind, Option<H264Profile>),
        size: FrameSize,
        contract: Option<FrameContract>,
        preference: DecoderPreference,
    ) -> bool {
        let (codec, profile) = stream;
        if codec == CodecKind::Tiles {
            return true;
        }
        self.decoders.iter().any(|factory| {
            factory.offers().iter().any(|offer| {
                offer.decodes(codec, profile)
                    && preference.allows(offer.hardware)
                    && offer.supports_size_rate(size, 0, contract)
            })
        })
    }
}

#[cfg(test)]
#[path = "bildkette_viewer_codec_task_tests.rs"]
mod bildkette_viewer_codec_task_tests;
