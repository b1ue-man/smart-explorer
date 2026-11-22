//! B-36 long-term references of the Linux NVENC session (review
//! 2026-11-19, as Windows' `windows/nvenc_ltr.rs`): the session's side of
//! `nvenc_shared::ltr` (see there).
use super::*;
use crate::screen::os::nvenc_shared::abi::Guid;
use crate::screen::os::nvenc_shared::ltr::{recover, short_term_dpb, LongTerm};

impl NvencSession {
    /// At open, before `nvEncInitializeEncoder`: long-term references where
    /// the session can keep them.
    pub(super) fn open_long_term(&mut self, guid: Guid) {
        let session = (&*self.api, self.encoder, guid);
        let (codec, fps) = (self.codec, self.rate.fps);
        self.long_term = LongTerm::open(session, &mut self.config, codec, fps, self.invalidation);
        if self.long_term.is_some() {
            self.history = RefHistory::new(short_term_dpb(codec, true));
        }
    }

    /// After the settings were written anew (a retune or its undo).
    pub(super) fn keep_long_term(&mut self) {
        if let Some(long_term) = self.long_term.as_mut() {
            long_term.reapply(&mut self.config, self.rate.fps);
        }
    }

    /// The long-term part of the picture now encoded; whether it is an IDR
    /// (asked, or owed after a failed check, round 11).
    pub(super) fn plan_long_term(&mut self, params: &mut NvEncPicParams, idr: bool) -> bool {
        let frame_id = self.frame_id;
        let Some(long_term) = self.long_term.as_mut() else {
            return idr;
        };
        let idr = long_term.idr_owed() || idr;
        long_term.plan(params, frame_id, idr);
        idr
    }

    /// After the bitstream: the check of a forced picture and an IDR the
    /// encoder made on its own (`LongTerm::used`, `LongTerm::coded`).
    pub(super) fn long_term_coded(&mut self, lock: &NvEncLockBitstream) -> io::Result<()> {
        let Some(long_term) = self.long_term.as_mut() else {
            return Ok(());
        };
        long_term.coded(lock.picture_type == NV_ENC_PIC_TYPE_IDR);
        long_term.used(lock.picture_type, lock.ltr_frame_bitmap)
    }

    /// The viewer confirmed `frame_id` (`ScreenEncoder::confirm_reference`).
    pub(super) fn confirm(&mut self, frame_id: u64) {
        if let Some(long_term) = self.long_term.as_mut() {
            long_term.confirm(frame_id);
        }
    }

    /// B-36 (`nvenc_shared::ltr::recover`): every picture after `last_good`
    /// is invalidated, the next one references `last_good` or older.
    pub(super) fn recover(&mut self, last_good: u64) -> Recovery {
        let invalidate = self.api.functions.invalidate_ref_frames;
        let (Some(invalidate), true) = (invalidate, self.invalidation) else {
            return Recovery::KeyframeNeeded;
        };
        let Some(lost) = recover(&mut self.history, self.long_term.as_mut(), last_good) else {
            return Recovery::KeyframeNeeded;
        };
        let _current = self.context.push();
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
