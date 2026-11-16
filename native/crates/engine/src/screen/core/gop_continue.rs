//! Continuation of a picture stream after a reset (plan 2026-11-14
//! „Bildkette“ B-36), the capability `gop_continue_v1` and the decisions of
//! both sides.
//!
//! Before B-36 every reset of a stale picture stream (`pacing.rs`, B-35)
//! cost a key frame of a new encoder on a new stream, and the viewer opened
//! a new decoder for every stream. With the capability:
//!
//! - The viewer announces `gop_continue: Some(1)` in `CLIENT_HELLO`. A host
//!   of v0.5.196 ignores the field (serde, no `deny_unknown_fields`), so it
//!   resets as before. A viewer of v0.5.196 never names it, so a new host
//!   resets as before towards it. Only a decoding side that keeps its
//!   decoder over a continuation may announce it (`ViewerRequest`).
//! - After a stale reset the host keeps its encoder when the stage passes
//!   the wish on (`HostStage::continues_references`, the in-process
//!   encoder) and asks it for a picture that references only the newest
//!   frame the viewer acknowledged on the stream it wrote
//!   (`ConfirmedReference`); an encoder that cannot answers with a key frame
//!   of a new encoder, as before (`ScreenEncoder::recover_references`).
//! - The new stream's first frame carries `FLAG_CONTINUE` instead of
//!   `FLAG_KEYFRAME`. The viewer's reader hands it on only after the older
//!   streams' records (`gop_stream_read.rs`), so every frame the viewer
//!   acknowledged reaches the decoder first.
//! - The viewer keeps its decoder and its references for such a stream when
//!   the decoder decoded every record of its chain (`DecoderContinuity`);
//!   otherwise it skips the stream, keeps showing the last good picture and
//!   asks for a key frame (`KEYFRAME_REQUEST`, the existing way).
//!
//! Per codec (plan „Bildkette“ P1): `gop_continue: Some(1)` still means
//! "every decoder of the offers continues". A viewer whose decoders differ
//! per codec (H.264 cannot, HEVC and AV1 can) leaves that field out and
//! names the continuing decoders in `gop_continue_decoders`; the host then
//! continues only when the decoder of the negotiated pairing is one of them
//! (`ContinueOffers`). A host of this batch before that field ignores it and
//! keeps the whole-session rule; a peer of v0.5.196 knows neither field.
use super::negotiate::{CodecChoice, CodecOffer};
use super::wire_binary::FrameRecord;
use super::wire_hello::ScreenClientHello;

/// Version of `gop_continue`, announced in `ScreenClientHello`.
pub const GOP_CONTINUE_V1: u16 = 1;

/// Whether the host may send continuation streams to this viewer for every
/// pairing (the whole-session announcement).
pub fn negotiated(hello: &ScreenClientHello) -> bool {
    hello.gop_continue == Some(GOP_CONTINUE_V1)
}

/// What the viewer announced about continuations: for every pairing, or for
/// the pairings whose decoder is one of `decoders`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContinueOffers {
    every: bool,
    decoders: Vec<CodecOffer>,
}

impl ContinueOffers {
    pub fn new(every: bool, decoders: Vec<CodecOffer>) -> Self {
        Self { every, decoders }
    }

    /// The announcement of `CLIENT_HELLO`.
    pub fn from_hello(hello: &ScreenClientHello) -> Self {
        Self::new(
            negotiated(hello),
            hello.gop_continue_decoders.clone().unwrap_or_default(),
        )
    }

    /// Whether the decoder of `choice` keeps its references over a
    /// continuation: the whole-session announcement, or a named decoder of
    /// the same codec and stage that decodes the pairing's profile.
    pub fn continues(&self, choice: &CodecChoice) -> bool {
        choice.codec.is_video()
            && (self.every
                || self.decoders.iter().any(|offer| {
                    offer.hardware == choice.decoder_hardware
                        && offer.decodes(choice.codec, choice.profile)
                }))
    }
}

/// What the host knows when a stream went stale (`Pipeline::tick`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResetFacts {
    /// The viewer announced `gop_continue_v1`.
    pub negotiated: bool,
    /// A video stream (`video` kind, H.264/HEVC/AV1, no hybrid regions).
    pub video: bool,
    /// A key frame is due anyway (request, new viewer, new size).
    pub keyframe_due: bool,
    /// An encoder is running whose chain the viewer decoded.
    pub encoder_running: bool,
    /// The stage passes the wish on to its encoder.
    pub stage_continues: bool,
    /// The newest frame the viewer can decode (`ConfirmedReference`).
    pub last_good: Option<u64>,
}

impl ResetFacts {
    /// The frame the next stream continues from; `None` resets as before
    /// (new encoder, key frame).
    pub fn continue_from(&self) -> Option<u64> {
        let continuable = self.negotiated
            && self.video
            && !self.keyframe_due
            && self.encoder_running
            && self.stage_continues;
        self.last_good.filter(|_| continuable)
    }
}

/// What a viewer does with the first record of a new picture stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamStart {
    /// A key frame: a new decoder (the previous one goes).
    OpenDecoder,
    /// A continuation of an intact chain: the decoder and its reference
    /// pictures stay and decode the record.
    KeepDecoder,
    /// A continuation the decoder cannot follow (a record of its chain
    /// failed or was skipped, or there is no decoder): skip the stream, keep
    /// showing the last good picture and ask for a key frame.
    RequestKeyframe,
}

/// The viewer's state of its decoder chain (B-36): intact while the decoder
/// decoded every record since its key frame. A stream that broke off
/// (`PictureEvent::Broken`) keeps the chain intact: the reader hands on
/// whole frames only, so the decoder never saw the lost part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecoderContinuity {
    intact: bool,
}

impl DecoderContinuity {
    /// The first record of a new picture stream.
    pub fn start(&mut self, record: &FrameRecord) -> StreamStart {
        if record.is_keyframe() {
            self.intact = true;
            StreamStart::OpenDecoder
        } else if record.is_continuation() && self.intact {
            StreamStart::KeepDecoder
        } else {
            self.intact = false;
            StreamStart::RequestKeyframe
        }
    }

    /// A record of the chain failed to decode or was not decoded (skipped,
    /// stale, dropped): no continuation may follow it before a key frame.
    pub fn broken(&mut self) {
        self.intact = false;
    }

    /// Whether a continuation would be decoded now.
    pub fn intact(&self) -> bool {
        self.intact
    }
}

#[cfg(test)]
#[path = "bildkette_recover_task_tests.rs"]
mod bildkette_recover_task_tests;
