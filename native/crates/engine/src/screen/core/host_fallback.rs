//! The encoder of a session without a picture (plan 2026-11-18
//! „Netzrate“): the watch of the first picture (`first_picture.rs`), a
//! pairing for a size the session's pairing cannot carry, and the clear
//! error when no encoder of the chosen codec is left (a chosen codec never
//! gives way to another codec or the tiles, `FallbackScope`).
use super::*;
use crate::screen::core::first_picture::{StageAnswer, FIRST_PICTURE_DEADLINE_US};
use crate::screen::core::pacing::codec_cap;

impl Pipeline {
    /// The stage answered a request without a picture: an encoder of a
    /// video codec that got pictures but delivered none for
    /// `FIRST_PICTURE_DEADLINE_US` counts as failed; a capture without a
    /// picture never does (`StageAnswer`).
    pub(super) fn on_silent_answer(
        &mut self,
        request: &StageRequest,
        answer: StageAnswer,
    ) -> Result<Produced, PipelineError> {
        if !answer.blames_encoder() {
            return Ok(Produced::Idle);
        }
        let watched = request.choice.codec.is_video()
            && request.keyframe
            && request.choice == self.choice
            && self.gop.is_none();
        if watched
            && self
                .quality_policy
                .first_picture
                .on_silent(request.choice, now_us())
        {
            let error = io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "no first picture from the encoder within {} ms",
                    FIRST_PICTURE_DEADLINE_US / 1000
                ),
            );
            return self.give_up_encoder(error, false);
        }
        Ok(Produced::Idle)
    }

    /// No encoder of the chosen codec is left: the session ends with the
    /// reason and the chosen codec instead of sending another one.
    pub(super) fn chosen_codec_failed(&mut self, error: io::Error) -> PipelineError {
        let text = format!(
            "chosen_codec_failed: {}: no encoder of the chosen codec is left: {error}",
            self.quality_policy.scope().key()
        );
        self.fallback_notice = Some(text.clone());
        PipelineError::encoder(io::Error::other(text))
    }

    /// The pairing's macroblock rate does not carry the picture even at one
    /// frame a second: another pairing of the scope that does, for now (the
    /// next mode returns to the session's pairing). `false` when none does.
    pub(super) fn fit_pairing_for_size(&mut self) -> bool {
        let scope = self.quality_policy.scope();
        let current = self.choice;
        let candidates: Vec<CodecChoice> = self
            .quality_policy
            .fallbacks
            .iter()
            .copied()
            .filter(|choice| scope.admits(choice))
            .collect();
        for candidate in candidates {
            self.choice = candidate;
            let fitted = self.fitted_size();
            let size = self.target_size();
            if fitted.width >= 2
                && fitted.height >= 2
                && codec_cap(self.max_fps, size, candidate.max_macroblocks_per_second).is_some()
            {
                self.quality.size = fitted;
                self.invalidate_reference();
                return true;
            }
        }
        self.choice = current;
        false
    }
}

/// „hardware“ or „software“: the encoder stage of `choice`.
pub(super) fn stage_name(choice: &CodecChoice) -> &'static str {
    if choice.encoder_hardware {
        "hardware"
    } else {
        "software"
    }
}
