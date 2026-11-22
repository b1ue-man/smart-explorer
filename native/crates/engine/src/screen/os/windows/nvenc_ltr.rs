//! B-36 long-term references of the NVENC session (review 2026-11-19,
//! Gemini round 8 finding 3.1). The DPB of short-term pictures that
//! `RefHistory` tracks holds a few frame intervals, while a stale stream is
//! noticed after 200 ms or more: 6 to 12 pictures at 30 to 60 fps, so the
//! confirmed picture has mostly left it. Two long-term references in
//! "LTR Per Picture" mode (`nvEncodeAPI.h` 12.1 at `enableLTR`) keep a
//! picture the viewer confirmed (`LtrSlots`, fed by `confirm`): after a
//! reset the next picture references only that one (`ltrUseFrames` with
//! its bit in `ltrUseFrameBitmap`) where no short-term picture is old
//! enough; the pictures after it reference it (one reference per picture,
//! `numRefL0` 1). Taken only where the session invalidates references
//! (`invalidation`, which sets that one reference), reports at least two
//! long-term references (`NV_ENC_CAPS_NUM_MAX_LTR_FRAMES`) and writes H.264
//! or HEVC (the AV1 member of API 12.1 has none).
//!
//! The header calls `ltrNumFrames` a guidance "not necessarily honored"
//! (faster presets keep fewer). So the picture after a reset is checked:
//! `NV_ENC_LOCK_BITSTREAM::ltrFrameBitmap` names the long-term references
//! it used; one that used none and is no intra picture fails (its bytes are
//! not sent, the stage opens a fresh encoder with a key frame) and turns the
//! long-term references off for the process (`IGNORED`).
//!
//! The long-term references take two places of the DPB of `RFI_DPB`: the
//! short-term pictures `RefHistory` counts are three. An IDR drops them
//! (`LtrSlots::key_frame`, also for an IDR the encoder made on its own,
//! Gemini 8/3.2, `coded`); the IDR itself is not marked, the next picture is.
use super::*;
use crate::screen::core::ref_recovery::LtrSlots;
use crate::screen::os::nvenc_shared::api::ltr_frames;
use crate::screen::os::nvenc_shared::codecs::{apply_ltr, ltr_pic_offsets};
use std::sync::atomic::{AtomicBool, Ordering};

/// The long-term references NVENC keeps (`LtrSlots` has two).
const FRAMES: u32 = 2;
/// `NV_ENC_PIC_TYPE_I`.
const NV_ENC_PIC_TYPE_I: u32 = 0x02;
/// A picture after a reset ignored its long-term reference (see above).
static IGNORED: AtomicBool = AtomicBool::new(false);

/// The long-term references of one session.
pub(super) struct LongTerm {
    slots: LtrSlots,
    /// The bits the picture now encoded was forced to (`ltrUseFrameBitmap`).
    forced: Option<u32>,
    /// That picture was asked to be an IDR.
    asked_idr: bool,
}

/// Writes `value` at byte `offset` of the picture's codec union (Windows
/// runs little-endian).
fn put(params: &mut NvEncPicParams, offset: usize, value: u32) {
    let word = &mut params.codec_pic_params[offset / 8];
    let shift = (offset % 8) * 8;
    *word = (*word & !(u64::from(u32::MAX) << shift)) | (u64::from(value) << shift);
}

impl<A: NvTable> Core<A> {
    /// At open: long-term references where the session can keep them.
    pub(super) fn open_long_term(&mut self, guid: crate::screen::os::nvenc_shared::abi::Guid) {
        if !self.invalidation
            || IGNORED.load(Ordering::Relaxed)
            || ltr_frames(&self.api, self.encoder, guid) < FRAMES
            || !apply_ltr(&mut self.config, self.codec, FRAMES)
        {
            return;
        }
        self.history = RefHistory::new(rfi_dpb(self.codec) - FRAMES);
        self.long_term = Some(LongTerm {
            slots: LtrSlots::new(self.rate.fps),
            forced: None,
            asked_idr: false,
        });
    }

    /// After `configure` wrote the settings anew (a retune).
    pub(super) fn keep_long_term(&mut self) {
        if let Some(long_term) = self.long_term.as_mut() {
            apply_ltr(&mut self.config, self.codec, FRAMES);
            long_term.slots.set_fps(self.rate.fps);
        }
    }

    /// The long-term part of the picture `frame_id` (`submit`).
    pub(super) fn plan_long_term(&mut self, params: &mut NvEncPicParams, frame_id: u64, idr: bool) {
        let codec = self.codec;
        let Some(long_term) = self.long_term.as_mut() else {
            return;
        };
        long_term.asked_idr = idr;
        long_term.forced = None;
        if idr {
            long_term.slots.key_frame();
            return;
        }
        let Some((flags_at, mark_at, use_at)) = ltr_pic_offsets(codec) else {
            return;
        };
        let plan = long_term.slots.plan(frame_id, false);
        let mut flags = 0;
        if let Some(index) = plan.mark {
            flags |= 1 << 2;
            put(params, mark_at, index);
        }
        if let Some(bits) = plan.force {
            flags |= 1 << 3;
            put(params, use_at, bits);
            long_term.forced = Some(bits);
        }
        put(params, flags_at, flags);
    }

    /// The check of a forced picture (see the module text).
    pub(super) fn long_term_used(&mut self, picture_type: u32, used: u32) -> io::Result<()> {
        let Some(forced) = self.long_term.as_mut().and_then(|l| l.forced.take()) else {
            return Ok(());
        };
        let intra = picture_type == NV_ENC_PIC_TYPE_I || picture_type == NV_ENC_PIC_TYPE_IDR;
        if used & forced != 0 || intra {
            return Ok(());
        }
        IGNORED.store(true, Ordering::Relaxed);
        self.long_term = None;
        Err(invalid(
            "NVENC did not reference the confirmed long-term picture; long-term references off",
        ))
    }

    /// Gemini 8/3.2: `keyframe` by the driver's picture type or by the
    /// stream's marks (`VendorSession::coded`); the picture is in `history`
    /// already (`encode`), an IDR again leaves it alone in there.
    pub(super) fn coded(&mut self, frame_id: u64, keyframe: bool) {
        if keyframe {
            self.history.encoded(frame_id, true);
        }
        if let Some(long_term) = self.long_term.as_mut().filter(|l| keyframe && !l.asked_idr) {
            long_term.slots.key_frame();
        }
    }

    /// The viewer confirmed `frame_id` (`ScreenEncoder::confirm_reference`).
    pub(super) fn confirm(&mut self, frame_id: u64) {
        if let Some(long_term) = self.long_term.as_mut() {
            long_term.slots.confirm(frame_id);
        }
    }

    /// B-36: a short-term picture not newer than `last_good` stays the
    /// reference (the newer ones are invalidated, as before); else, with
    /// long-term references, the newest marked picture not newer than it.
    pub(super) fn recover_references(&mut self, last_good: u64) -> Recovery {
        let invalidate = self.api.table().invalidate_ref_frames;
        let (Some(invalidate), true) = (invalidate, self.invalidation) else {
            return Recovery::KeyframeNeeded;
        };
        let lost = match self.history.invalidate_after(last_good) {
            Some(lost) => {
                // Marked pictures newer than `last_good` never count again.
                if let Some(long_term) = self.long_term.as_mut() {
                    long_term.slots.drop_after(last_good);
                }
                lost
            }
            None if self
                .long_term
                .as_mut()
                .is_some_and(|l| l.slots.recover(last_good)) =>
            {
                // Every short-term picture is newer than `last_good`.
                self.history.drain()
            }
            None => return Recovery::KeyframeNeeded,
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
