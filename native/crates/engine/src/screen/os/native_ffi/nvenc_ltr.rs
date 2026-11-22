//! B-36 long-term references of an NVENC session on both desktop platforms
//! (review 2026-11-19, Gemini round 8 finding 3.1; Windows
//! `windows/nvenc_ltr.rs`, Linux `linux_os/nvenc_ltr.rs`). The DPB of
//! short-term pictures that `RefHistory` tracks holds a few frame
//! intervals, while a stale stream is noticed after 200 ms or more: 6 to 12
//! pictures at 30 to 60 fps, so the confirmed picture has mostly left it.
//! Two long-term references in "LTR Per Picture" mode (`nvEncodeAPI.h` 12.1
//! at `enableLTR`) keep a picture the viewer confirmed (`LtrSlots`, fed by
//! `confirm`): after a reset the next picture references only that one
//! (`ltrUseFrames` with its bit in `ltrUseFrameBitmap`) where no short-term
//! picture is old enough; the pictures after it reference it (one reference
//! per picture, `numRefL0` 1). Taken only where the session invalidates
//! references (`invalidation`, which sets that one reference), reports at
//! least two long-term references (`NV_ENC_CAPS_NUM_MAX_LTR_FRAMES`) and
//! writes H.264 or HEVC (the AV1 member of API 12.1 has none).
//!
//! The header calls `ltrNumFrames` a guidance "not necessarily honored"
//! (faster presets keep fewer). So the picture after a reset is checked
//! (`used`): `NV_ENC_LOCK_BITSTREAM::ltrFrameBitmap` names the long-term
//! references it used; one that used none and is no intra picture fails
//! (its bytes are not sent, the stage opens a fresh encoder with a key
//! frame) and turns the long-term references off for the process
//! (`IGNORED`).
//!
//! The long-term references take two places of the DPB of `RFI_DPB`: the
//! short-term pictures `RefHistory` counts are three (`short_term_dpb`). An
//! IDR drops them (`LtrSlots::key_frame`, also an IDR the encoder made on
//! its own, Gemini 8/3.2, `coded`); the IDR itself is not marked, the next
//! picture is.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::abi::{Guid, NvEncConfig, NvEncPicParams, Ptr};
use super::api::{ltr_frames, NvTable, NV_ENC_PIC_TYPE_IDR};
use super::codecs::{apply_ltr, ltr_pic_offsets, rfi_dpb};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::ref_recovery::{LtrSlots, RefHistory};
use crate::screen::core::wire::invalid;

/// The long-term references NVENC keeps (`LtrSlots` has two).
const FRAMES: u32 = 2;
/// `NV_ENC_PIC_TYPE_I`.
const NV_ENC_PIC_TYPE_I: u32 = 0x02;
/// A picture after a reset ignored its long-term reference (see above).
static IGNORED: AtomicBool = AtomicBool::new(false);

/// The long-term references of one session.
pub(in crate::screen::os) struct LongTerm {
    codec: CodecKind,
    slots: LtrSlots,
    /// The bits the picture now encoded was forced to (`ltrUseFrameBitmap`).
    forced: Option<u32>,
    /// That picture was asked to be an IDR.
    asked_idr: bool,
}

/// Writes `value` at byte `offset` of the picture's codec union (both
/// platforms run little-endian).
fn put(params: &mut NvEncPicParams, offset: usize, value: u32) {
    let word = &mut params.codec_pic_params[offset / 8];
    let shift = (offset % 8) * 8;
    *word = (*word & !(u64::from(u32::MAX) << shift)) | (u64::from(value) << shift);
}

/// The DPB `RefHistory` counts with `long_term` (see the module text).
pub(in crate::screen::os) fn short_term_dpb(codec: CodecKind, long_term: bool) -> u32 {
    rfi_dpb(codec) - if long_term { FRAMES } else { 0 }
}

impl LongTerm {
    /// At open, before `nvEncInitializeEncoder`: the settings in `config`
    /// where the session can keep long-term references.
    pub(in crate::screen::os) fn open<A: NvTable + ?Sized>(
        (api, encoder, guid): (&A, Ptr, Guid),
        config: &mut NvEncConfig,
        codec: CodecKind,
        fps: u32,
        invalidation: bool,
    ) -> Option<Self> {
        if !invalidation
            || IGNORED.load(Ordering::Relaxed)
            || ltr_frames(api, encoder, guid) < FRAMES
            || !apply_ltr(config, codec, FRAMES)
        {
            return None;
        }
        Some(Self {
            codec,
            slots: LtrSlots::new(fps),
            forced: None,
            asked_idr: false,
        })
    }

    /// After the settings were written anew (a retune).
    pub(in crate::screen::os) fn reapply(&mut self, config: &mut NvEncConfig, fps: u32) {
        apply_ltr(config, self.codec, FRAMES);
        self.slots.set_fps(fps);
    }

    /// The long-term part of the picture `frame_id`.
    pub(in crate::screen::os) fn plan(
        &mut self,
        params: &mut NvEncPicParams,
        frame_id: u64,
        idr: bool,
    ) {
        self.asked_idr = idr;
        self.forced = None;
        if idr {
            self.slots.key_frame();
            return;
        }
        let Some((flags_at, mark_at, use_at)) = ltr_pic_offsets(self.codec) else {
            return;
        };
        let plan = self.slots.plan(frame_id, false);
        let mut flags = 0;
        if let Some(index) = plan.mark {
            flags |= 1 << 2;
            put(params, mark_at, index);
        }
        if let Some(bits) = plan.force {
            flags |= 1 << 3;
            put(params, use_at, bits);
            self.forced = Some(bits);
        }
        put(params, flags_at, flags);
    }

    /// The check of a forced picture (see the module text); after an error
    /// the session drops its `LongTerm`.
    pub(in crate::screen::os) fn used(&mut self, picture_type: u32, used: u32) -> io::Result<()> {
        let Some(forced) = self.forced.take() else {
            return Ok(());
        };
        let intra = picture_type == NV_ENC_PIC_TYPE_I || picture_type == NV_ENC_PIC_TYPE_IDR;
        if used & forced != 0 || intra {
            return Ok(());
        }
        IGNORED.store(true, Ordering::Relaxed);
        Err(invalid(
            "NVENC did not reference the confirmed long-term picture; long-term references off",
        ))
    }

    /// Gemini 8/3.2: the picture was a key frame by the driver's picture
    /// type or by the stream's marks; one not asked for drops the slots.
    pub(in crate::screen::os) fn coded(&mut self, keyframe: bool) {
        if keyframe && !self.asked_idr {
            self.slots.key_frame();
        }
    }

    /// The viewer confirmed `frame_id` (`ScreenEncoder::confirm_reference`).
    pub(in crate::screen::os) fn confirm(&mut self, frame_id: u64) {
        self.slots.confirm(frame_id);
    }
}

/// B-36: the pictures to invalidate so the next one references `last_good`
/// or older; `None`: a key frame is needed. A short-term picture not newer
/// than `last_good` stays the reference (the newer ones are invalidated, as
/// before); else, with long-term references, the newest marked picture not
/// newer than it, every short-term picture being newer.
pub(in crate::screen::os) fn recover(
    history: &mut RefHistory,
    long_term: Option<&mut LongTerm>,
    last_good: u64,
) -> Option<Vec<u64>> {
    if let Some(lost) = history.invalidate_after(last_good) {
        // Marked pictures newer than `last_good` never count again.
        if let Some(long_term) = long_term {
            long_term.slots.drop_after(last_good);
        }
        return Some(lost);
    }
    // `recover` changes the slots: outside a match guard.
    let recovered = long_term.is_some_and(|long_term| long_term.slots.recover(last_good));
    recovered.then(|| history.drain())
}
