//! B-36 long-term references of the Windows NVENC session (review
//! 2026-11-19, Gemini round 8 finding 3.1): the session's side of
//! `nvenc_shared::ltr` (see there).
use super::*;
use crate::screen::os::nvenc_shared::abi::Guid;
use crate::screen::os::nvenc_shared::ltr::{recover, short_term_dpb, LongTerm};

impl<A: NvTable> Core<A> {
    /// At open: long-term references where the session can keep them.
    pub(super) fn open_long_term(&mut self, guid: Guid) {
        let session = (&self.api, self.encoder, guid);
        let (codec, fps) = (self.codec, self.rate.fps);
        self.long_term = LongTerm::open(session, &mut self.config, codec, fps, self.invalidation);
        if self.long_term.is_some() {
            self.history = RefHistory::new(short_term_dpb(codec, true));
        }
    }

    /// After `configure` wrote the settings anew (a retune).
    pub(super) fn keep_long_term(&mut self) {
        if let Some(long_term) = self.long_term.as_mut() {
            long_term.reapply(&mut self.config, self.rate.fps);
        }
    }

    /// The long-term part of the picture `frame_id` (`submit`).
    pub(super) fn plan_long_term(&mut self, params: &mut NvEncPicParams, frame_id: u64, idr: bool) {
        if let Some(long_term) = self.long_term.as_mut() {
            long_term.plan(params, frame_id, idr);
        }
    }

    /// The check of a forced picture (`LongTerm::used`).
    pub(super) fn long_term_used(&mut self, picture_type: u32, used: u32) -> io::Result<()> {
        let checked = self
            .long_term
            .as_mut()
            .map_or(Ok(()), |long_term| long_term.used(picture_type, used));
        if checked.is_err() {
            self.long_term = None;
        }
        checked
    }

    /// Gemini 8/3.2: `keyframe` by the driver's picture type or by the
    /// stream's marks (`VendorSession::coded`); the picture is in `history`
    /// already (`encode`), an IDR again leaves it alone in there.
    pub(super) fn coded(&mut self, frame_id: u64, keyframe: bool) {
        if keyframe {
            self.history.encoded(frame_id, true);
        }
        if let Some(long_term) = self.long_term.as_mut() {
            long_term.coded(keyframe);
        }
    }

    /// The viewer confirmed `frame_id` (`ScreenEncoder::confirm_reference`).
    pub(super) fn confirm(&mut self, frame_id: u64) {
        if let Some(long_term) = self.long_term.as_mut() {
            long_term.confirm(frame_id);
        }
    }

    /// B-36 (`nvenc_shared::ltr::recover`).
    pub(super) fn recover_references(&mut self, last_good: u64) -> Recovery {
        let invalidate = self.api.table().invalidate_ref_frames;
        let (Some(invalidate), true) = (invalidate, self.invalidation) else {
            return Recovery::KeyframeNeeded;
        };
        let Some(lost) = recover(&mut self.history, self.long_term.as_mut(), last_good) else {
            return Recovery::KeyframeNeeded;
        };
        for frame in lost {
            // SAFETY: valid session; `frame` is the timestamp of a picture
            // it encoded (`input_time_stamp`).
            if unsafe { invalidate(self.encoder, frame) } != NV_ENC_SUCCESS {
                return Recovery::KeyframeNeeded;
            }
        }
        Recovery::Continued
    }
}
