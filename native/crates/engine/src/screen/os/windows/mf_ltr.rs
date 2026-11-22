//! B-36 long-term references of the hardware Media Foundation encoder
//! (review 2026-11-19, Gemini round 8 finding 3.1), H.264 only: the
//! Microsoft documentation describes these controls for "H.264/AVC
//! encoders" alone.
//!
//! - `CODECAPI_AVEncVideoLTRBufferControl` (`mf_media::configure_hardware`):
//!   trust mode 1, two frames under the application's control. "At each IDR
//!   frame the encoder must automatically mark the frames following the IDR
//!   frame (and including the IDR frame) as LTR frames" while fewer than two
//!   are marked; which index it takes is not said.
//! - `CODECAPI_AVEncVideoMarkLTRFrame`: the value is the `LongTermFrameIdx`
//!   of the current frame; no second request while one is pending.
//! - `CODECAPI_AVEncVideoUseLTRFrame`: bits 0–15 the LTR frames this frame
//!   may use, bits 16–31 = 1: "it shall not use short term reference frames
//!   in encoding order older than the current frame" nor LTR frames the
//!   control does not name, so the pictures after it reference only it and
//!   newer ones.
//! - `MFSampleExtension_LongTermReferenceFrameInfo` on every output sample
//!   while the application controls LTR frames: bits 0–15 the
//!   `LongTermFrameIdx` the frame was marked with (0xffff: none), bits
//!   16–31 the LTR frames it used.
//!
//! So the indices are known from the output, the encoder's own marks after
//! an IDR included (`LtrSlots::marked` records each output's mark with the
//! frame id of the call that gave it: never older than the frame marked).
//! The marks every quarter second and the choice of the confirmed slot are
//! `LtrSlots` (as AMF and NVENC). An output without the attribute ends the
//! long-term references of the session (its marks are unknown then: key
//! frames of the running encoder as before); a picture forced to a slot
//! that used none and is no key frame fails, so the stage opens a fresh
//! encoder with a key frame (as NVENC's check).
use super::*;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::ref_recovery::LtrSlots;
use windows::core::VARIANT;
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncVideoMarkLTRFrame, CODECAPI_AVEncVideoUseLTRFrame, ICodecAPI, IMFSample,
    MFSampleExtension_LongTermReferenceFrameInfo,
};

/// The long-term references `LtrSlots` keeps.
const FRAMES: u16 = 2;
/// `MFSampleExtension_LongTermReferenceFrameInfo` of a frame not marked.
const NOT_MARKED: u32 = 0xffff;

/// The long-term references of one encoder.
pub(super) struct MfLtr {
    slots: LtrSlots,
    /// The bits the picture now encoded was forced to.
    forced: Option<u32>,
    /// The slot the picture now encoded was asked to be marked in.
    marking: Option<u32>,
}

impl MfLtr {
    /// With two LTR frames under the application's control on H.264.
    pub(super) fn open(setup: &EncoderSetup, ltr_frames: u16) -> Option<Self> {
        (setup.codec == CodecKind::H264 && ltr_frames == FRAMES).then(|| Self {
            slots: LtrSlots::new(setup.fps),
            forced: None,
            marking: None,
        })
    }

    /// The marks and the forced reference of the picture `frame_id`, set on
    /// the encoder before its input; `key`: it is a key frame (marked as
    /// LTR 0, as before). `false`: a forced reference was refused, so the
    /// picture must be a key frame (round 10).
    pub(super) fn plan(&mut self, api: Option<&ICodecAPI>, frame_id: u64, key: bool) -> bool {
        let plan = self.slots.plan(frame_id, key);
        let set = |name, value: u32| {
            api.is_some_and(|api| {
                super::super::mf_media::set_value(api, name, VARIANT::from(value))
            })
        };
        if let Some(index) = plan.mark {
            // Round 10, 4: a refused request leaves the slot unknown.
            if set(&CODECAPI_AVEncVideoMarkLTRFrame, index) {
                self.marking = Some(index);
            } else {
                self.slots.unmark(index as usize);
            }
        }
        // A forced picture without output stays pending in the encoder (no
        // queued request) and is checked at the next output.
        match plan.force {
            Some(bits) if set(&CODECAPI_AVEncVideoUseLTRFrame, (1 << 16) | bits) => {
                self.forced = Some(bits);
                true
            }
            Some(_) => false,
            None => true,
        }
    }

    /// The viewer confirmed `frame_id` (`ScreenEncoder::confirm_reference`).
    pub(super) fn confirm(&mut self, frame_id: u64) {
        self.slots.confirm(frame_id);
    }
}

impl AsyncEncoder {
    /// The output of the picture `frame_id` (see the module text).
    pub(super) fn long_term_output(
        &mut self,
        sample: &IMFSample,
        keyframe: bool,
        frame_id: u64,
    ) -> io::Result<()> {
        let Some(long_term) = self.long_term.as_mut() else {
            return Ok(());
        };
        if keyframe {
            long_term.slots.key_frame();
        }
        let forced = long_term.forced.take();
        // SAFETY: attribute read on an owned output sample.
        let info = unsafe { sample.GetUINT32(&MFSampleExtension_LongTermReferenceFrameInfo) };
        let Ok(info) = info else {
            eprintln!(
                "screen encoder {}: no LTR frame info, key frames after a reset",
                self.setup.name
            );
            self.long_term = None;
            return Ok(());
        };
        let index = info & 0xffff;
        // Round 10, 4: a planned mark the output does not report is no mark.
        if let Some(asked) = long_term.marking.take().filter(|asked| *asked != index) {
            long_term.slots.unmark(asked as usize);
        }
        if index != NOT_MARKED && !long_term.slots.marked(index as usize, frame_id) {
            self.long_term = None;
            return Ok(());
        }
        match forced {
            // Round 10, 1: the picture is skipped and the running encoder
            // gives a key frame next; no GPU-path failure, no restart.
            Some(bits) if (info >> 16) & bits == 0 && !keyframe => {
                self.long_term = None;
                self.restarted = true;
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "the encoder did not reference the confirmed long-term picture",
                ))
            }
            _ => Ok(()),
        }
    }

    /// B-36: the next picture references only the newest marked picture not
    /// newer than `last_good` (`CODECAPI_AVEncVideoUseLTRFrame`).
    pub(super) fn recover(&mut self, last_good: u64) -> Recovery {
        if self.frames > 0
            && self
                .long_term
                .as_mut()
                .is_some_and(|l| l.slots.recover(last_good))
        {
            Recovery::Continued
        } else {
            Recovery::KeyframeNeeded
        }
    }
}
