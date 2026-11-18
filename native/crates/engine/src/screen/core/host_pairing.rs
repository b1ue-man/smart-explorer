//! The negotiated pairing of a running session and its change by
//! `SET_DECODERS` (plan 2026-11-14 „Bildkette“ P1, `set_decoders.rs`), and
//! the viewer's continuation announcement per decoder (B-36,
//! `gop_continue::ContinueOffers`).
//!
//! A new pairing goes through the path `SET_MODE` takes for a new codec
//! choice (`set_quality_mode`): the quality policy gets the new pairing as
//! its original choice, and a changed effective choice invalidates the
//! reference, so the next frame is a key frame of a new encoder in a new
//! picture group. Under „Verlustfrei“ the effective choice stays the tiles
//! and nothing restarts; the new pairing applies when the switch goes off.
use super::quality::FallbackScope;
use super::*;
use crate::screen::core::gop_continue::ContinueOffers;
use crate::screen::core::negotiate::{negotiate_chosen, CodecOffer, CodecTrust};
use crate::screen::core::set_decoders::{renegotiate_for, SetDecoders};

/// What the session knows to negotiate again.
pub(super) struct PairingState {
    /// The host's encoders as named in `HELLO_OK` (`set_pairing`).
    encoders: Vec<CodecOffer>,
    /// The negotiated pairing and its 4:4:4 alternative.
    current: CodecChoice,
    sharp: Option<CodecChoice>,
    /// The frame rate of the viewer's hello (`QualityPolicy::new`).
    viewer_fps: u32,
    /// The session opened with a video codec (its output plan is video).
    video_session: bool,
    /// What the viewer announced about continuations (B-36).
    pub(super) continuation: ContinueOffers,
    /// D6 (plan 2026-11-17 P3): the host's trust of the newer codecs.
    trust: CodecTrust,
}

impl PairingState {
    pub(super) fn new(choice: CodecChoice, viewer_fps: u32) -> Self {
        Self {
            encoders: Vec::new(),
            current: choice,
            sharp: None,
            viewer_fps,
            video_session: choice.codec.is_video(),
            continuation: ContinueOffers::default(),
            trust: CodecTrust::ALL,
        }
    }

    pub(super) fn on_codec_fallback(&mut self, failed: CodecKind, next: CodecChoice) {
        self.encoders.retain(|offer| offer.codec != failed);
        self.current = next;
        self.sharp = None;
    }
}

/// Whether two pairings use the same encoder (codec, profile, stage).
fn same_encoder(a: &CodecChoice, b: &CodecChoice) -> bool {
    a.codec == b.codec && a.profile == b.profile && a.encoder_hardware == b.encoder_hardware
}

impl Pipeline {
    /// At the opening (`host_server`): the host's encoders for a later
    /// `SET_DECODERS`, the negotiated 4:4:4 alternative (also given to
    /// `set_sharp_choice`) and the viewer's continuation announcement of
    /// its hello.
    pub fn set_pairing(
        &mut self,
        encoders: Vec<CodecOffer>,
        sharp: Option<CodecChoice>,
        continuation: ContinueOffers,
    ) {
        self.pairing.encoders = encoders;
        self.pairing.sharp = sharp;
        self.pairing.continuation = continuation;
    }

    /// D6 (plan 2026-11-17 P3): `SET_DECODERS` ranks with the trust of the
    /// opening.
    pub fn set_codec_trust(&mut self, trust: CodecTrust) {
        self.pairing.trust = trust;
    }

    /// Whether the decoder of the running pairing keeps its references over
    /// a continuation (B-36 per codec).
    pub(super) fn continuation_offered(&self) -> bool {
        self.pairing.continuation.continues(&self.choice)
    }

    /// `SET_DECODERS`: negotiates again; `true` when the pairing changed.
    /// The continuation announcement of the frame applies either way. Plan
    /// 2026-11-18 „Netzrate“: under the viewer's new choice; a chosen codec
    /// without a pairing ends the session at the next tick with its reason.
    pub fn set_decoders(&mut self, set: &SetDecoders) -> bool {
        self.pairing.continuation = set.continuation();
        let next = match renegotiate_for(
            &self.pairing.encoders,
            &set.decoders,
            self.negotiated_contract,
            self.pairing.video_session,
            self.pairing.trust,
        ) {
            Ok(next) => next,
            Err(refused) => {
                self.quality_policy.pending_error = Some(refused.to_string());
                return false;
            }
        };
        let Some(next) = next else {
            if self.continue_from.is_some() && !self.continuation_offered() {
                self.reset_stream();
            }
            return false;
        };
        if let Some(limit) = next.tile_limit {
            self.tile_limit = Some(limit);
        }
        self.quality_policy.mode.chosen = next.preference;
        if next.choice == self.pairing.current && next.sharp == self.pairing.sharp {
            let failed = &self.quality_policy.failed_codecs;
            let fallbacks = next
                .fallbacks
                .into_iter()
                .filter(|choice| !failed.contains(&choice.codec))
                .collect();
            self.quality_policy.fallbacks = fallbacks;
            if self.continue_from.is_some() && !self.continuation_offered() {
                self.reset_stream();
            }
            return false;
        }
        // An encoder that really failed stays given up; another one may try.
        if self.quality_policy.encoder_failed {
            let current_codec = self.pairing.current.codec;
            self.quality_policy.note_failed_codec(current_codec);
        }
        let failed_codecs = std::mem::take(&mut self.quality_policy.failed_codecs);
        let choices = negotiate_chosen(
            &self.pairing.encoders,
            &set.decoders,
            next.preference,
            self.negotiated_contract,
            self.pairing.trust,
        )
        .unwrap_or_default();
        let mut fallbacks = codec_fallbacks(&choices, &next.choice);
        fallbacks.retain(|choice| !failed_codecs.contains(&choice.codec));
        let mut chosen = next.choice;
        let mut sharp = next.sharp;
        let mut failed =
            self.quality_policy.encoder_failed && same_encoder(&chosen, &self.pairing.current);
        if failed_codecs.contains(&chosen.codec) {
            sharp = None;
            if let Some(replacement) = (!fallbacks.is_empty()).then(|| fallbacks.remove(0)) {
                chosen = replacement;
                failed = false;
            } else {
                failed = true;
            }
        }
        // A chosen codec whose encoders all failed here never becomes the
        // tiles (plan 2026-11-18): the session ends with the reason.
        let scope = FallbackScope::of(next.preference, self.quality_policy.mode.sharpness);
        let refused = (failed && !scope.tiles_allowed()).then(|| {
            format!(
                "chosen_codec_failed: {}: no encoder of the chosen codec is left",
                scope.key()
            )
        });
        self.pairing.current = next.choice;
        self.pairing.sharp = next.sharp;
        let mode = self.quality_policy.mode;
        let mut policy = QualityPolicy::new(chosen, self.pairing.viewer_fps);
        policy.mode = mode;
        policy.encoder_failed = failed && refused.is_none();
        // W4: the pairing's fallbacks without the codecs that failed here.
        policy.fallbacks = fallbacks;
        policy.failed_codecs = failed_codecs;
        policy.pending_error = refused;
        self.quality_policy = policy;
        // `set_sharp_choice` keeps an alternative of the running codec.
        let effective = self.choice;
        self.choice = chosen;
        self.set_sharp_choice(sharp);
        self.choice = effective;
        // Under „Verlustfrei“ the tiles stay the effective choice: nothing
        // restarts (not even an idle stage), the pairing waits in the policy.
        if self.quality_choice() != effective {
            self.set_quality_mode(mode);
        }
        if self.continue_from.is_some() && !self.continuation_offered() {
            self.reset_stream();
        }
        true
    }
}

#[cfg(test)]
#[path = "bildkette_proto_pairing_task_tests.rs"]
mod bildkette_proto_pairing_task_tests;
