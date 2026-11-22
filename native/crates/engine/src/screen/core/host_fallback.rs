//! The encoder of a session without a picture (plan 2026-11-18
//! „Netzrate“): the watch of the first picture (`first_picture.rs`), a
//! pairing for a size the session's pairing cannot carry, and the clear
//! error when no encoder of the chosen codec is left (a chosen codec never
//! gives way to another codec or the tiles, `FallbackScope`).
use super::*;
use crate::screen::core::codec_reason_text::ENCODER_NOTICE;
use crate::screen::core::first_picture::{StageAnswer, FIRST_PICTURE_DEADLINE_US};
use crate::screen::core::pacing::codec_cap;

/// Reasons that wait for a `STATE` frame at most (`queue_notice`).
const MAX_QUEUED_NOTICES: usize = 8;

impl Pipeline {
    /// Why the session's picture stream (and with it often its encoder)
    /// starts anew, into the screen host log (measurement of 2026-11-19: a
    /// new encoder about once a second and no line that said why).
    pub(super) fn note_restart(&self, why: &str) {
        let line = format!("screen stream: {why}");
        // The unit tests write no file of the account they run under.
        #[cfg(not(test))]
        crate::screen::os::host_worker_client::host_log::note(None, &line);
        #[cfg(test)]
        let _ = line;
    }

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
        self.note_encoder(text.clone());
        PipelineError::encoder(io::Error::other(text))
    }

    /// A failure, fallback or end of the session's encoder (measurement of
    /// 2026-11-18: such a reason reached neither the host log nor, under
    /// `se screen bench`, the viewer): into the screen host log with its
    /// reason and, as the next `STATE` reason, to the viewer; the error that
    /// ends the session carries the same text as its detail.
    pub(super) fn note_encoder(&mut self, text: String) {
        let line = format!("screen encoder: {text}");
        // The unit tests write no file of the account they run under.
        #[cfg(not(test))]
        crate::screen::os::host_worker_client::host_log::note(None, &line);
        #[cfg(test)]
        eprintln!("{line}");
        // One reason per `STATE` frame (the viewers read it by its prefix).
        // Review 2026-11-19 (V2): a later reason of the same run replaced a
        // stage's notice, which the stage gives only once. A codec reason
        // goes first, the notice follows with the next run
        // (`promote_queued_notice`); of two codec reasons the later one
        // counts, as before.
        let notice = |reason: &str| reason.starts_with(ENCODER_NOTICE);
        match self.fallback_notice.take() {
            Some(earlier) if notice(&text) => {
                self.fallback_notice = Some(earlier);
                self.queue_notice(text);
            }
            Some(earlier) if notice(&earlier) => {
                self.queue_notice(earlier);
                self.fallback_notice = Some(text);
            }
            _ => self.fallback_notice = Some(text),
        }
    }

    fn queue_notice(&mut self, text: String) {
        let queue = &mut self.quality_policy.queued_notices;
        if queue.len() >= MAX_QUEUED_NOTICES {
            queue.remove(0);
        }
        queue.push(text);
    }

    /// A reason that waited (`note_encoder`) takes the free slot of this run.
    pub(super) fn promote_queued_notice(&mut self) {
        if self.fallback_notice.is_none() && !self.quality_policy.queued_notices.is_empty() {
            self.fallback_notice = Some(self.quality_policy.queued_notices.remove(0));
        }
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
                self.note_restart("another pairing carries the picture size: new encoder");
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
