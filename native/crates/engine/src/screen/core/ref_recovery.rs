//! Recovery after a lost picture stream without a key frame (plan 2026-11-14
//! „Bildkette“ B-36): what an encoder reports after the wish to reference
//! the last picture the viewer confirmed (`Recovery`), the bookkeeping of
//! the encoders that can (`RefHistory` for NVENC, `LtrSlots` for AMF; moved
//! here from `os/windows/vendor_encode.rs`, so the Linux NVENC, VA-API and
//! MediaCodec paths can use them), and on the host the newest picture the
//! viewer can still decode (`ConfirmedReference`).
//!
//! Which picture is safe as a reference: `FRAME_ACK` means „arrived“, not
//! „decoded“: the viewer's reader sends it right after the last record of a
//! frame, before the decoder sees it (`gop_stream.rs`, `read_picture_stream`).
//! A picture stream is one ordered, reliable QUIC stream that starts with a
//! key frame or with a continuation of a confirmed picture, so the
//! acknowledgement of frame `f` of the current stream means that `f` and
//! every earlier frame of that stream arrived; with the reader's barrier
//! (a continuation stream is delivered only after the older streams were)
//! they reach the decoder in order before the continuation. Only
//! acknowledgements of the stream the host is writing count: a late one of
//! a reset stream names a picture the encoder may already have invalidated.
//! `PRESENTED` (decoded and shown) would be stronger, but exists only with
//! the v2 frame contract and only for pictures the viewer shows; a decode
//! error is answered by the viewer's key frame request instead, which drops
//! the confirmed picture here (`ConfirmedReference::restart`).
use std::collections::VecDeque;

/// What the encoder did with the wish to continue after a stream reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// The next picture references `last_good` or older: no key frame.
    Continued,
    /// The encoder cannot reach a confirmed picture: key frame as today.
    KeyframeNeeded,
}

/// NVENC (B-36): the pictures in the encoder's DPB since the last IDR
/// (Sunshine `nvenc_base.cpp:799-834`: a range at least as large as the DPB
/// needs an IDR; otherwise every picture after the last good one is
/// invalidated with `NvEncInvalidateRefFrames`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefHistory {
    dpb: usize,
    frames: VecDeque<u64>,
}

impl RefHistory {
    pub fn new(dpb: u32) -> Self {
        Self {
            dpb: dpb.max(1) as usize,
            frames: VecDeque::new(),
        }
    }

    pub fn encoded(&mut self, frame_id: u64, idr: bool) {
        if idr {
            self.frames.clear();
        }
        self.frames.push_back(frame_id);
        while self.frames.len() > self.dpb {
            self.frames.pop_front();
        }
    }

    /// The pictures to invalidate so the next one references `last_good`
    /// or older; `None` when no such picture is in the DPB.
    pub fn invalidate_after(&mut self, last_good: u64) -> Option<Vec<u64>> {
        if !self.frames.iter().any(|frame| *frame <= last_good) {
            return None;
        }
        let lost = self
            .frames
            .iter()
            .copied()
            .filter(|frame| *frame > last_good)
            .collect();
        self.frames.retain(|frame| *frame <= last_good);
        Some(lost)
    }

    /// Every picture it counts, which leave it (to invalidate them when a
    /// long-term reference takes over, `LtrSlots::recover`).
    pub fn drain(&mut self) -> Vec<u64> {
        self.frames.drain(..).collect()
    }
}

/// AMF and NVENC (B-36): two long-term references (AMF `MaxOfLTRFrames` 2,
/// NVENC `ltrNumFrames` 2), marked every quarter second and on every IDR;
/// after a reset the newest one not newer than the last confirmed picture
/// becomes the only reference of the next picture (AMF
/// `ForceLTRReferenceBitfield`, NVENC `ltrUseFrames`).
///
/// Gemini 8/3.1: once the viewer's acknowledgements reach the encoder
/// (`confirm`), the slot that holds the newest confirmed picture is kept and
/// only the other slot is marked anew, and only once its picture was
/// confirmed as well (or it is empty). So a confirmed long-term reference
/// exists whatever the round trip and the time a stale stream needs to be
/// noticed, where the DPB of short-term pictures (`RefHistory`) holds only
/// a few frame intervals. Without acknowledgements the slots alternate as
/// before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LtrSlots {
    interval: u64,
    slots: [Option<u64>; 2],
    next: usize,
    since: u64,
    forced: Option<usize>,
    /// The newest picture the viewer confirmed (`confirm`).
    confirmed: Option<u64>,
}

/// What one picture does with the long-term references.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LtrPlan {
    /// `MarkCurrentWithLTRIndex`.
    pub mark: Option<u32>,
    /// `ForceLTRReferenceBitfield`.
    pub force: Option<u32>,
}

impl LtrSlots {
    pub fn new(fps: u32) -> Self {
        Self {
            interval: Self::interval_for(fps),
            slots: [None; 2],
            next: 0,
            since: 0,
            forced: None,
            confirmed: None,
        }
    }

    fn interval_for(fps: u32) -> u64 {
        u64::from((fps / 4).max(1))
    }

    pub fn set_fps(&mut self, fps: u32) {
        self.interval = Self::interval_for(fps);
    }

    pub fn plan(&mut self, frame_id: u64, idr: bool) -> LtrPlan {
        if idr {
            self.key_frame();
        }
        let force = self.forced.take();
        self.since += 1;
        let mut plan = LtrPlan {
            mark: None,
            force: force.map(|slot| 1 << slot),
        };
        let target = self.target().filter(|slot| Some(*slot) != force);
        if let Some(slot) = target.filter(|_| self.since >= self.interval) {
            self.slots[slot] = Some(frame_id);
            plan.mark = Some(slot as u32);
            self.next = slot ^ 1;
            self.since = 0;
        }
        plan
    }

    /// Marked pictures newer than `last_good` were lost with a reset
    /// stream: never a reference, never confirmed later.
    pub fn drop_after(&mut self, last_good: u64) {
        for slot in &mut self.slots {
            if slot.is_some_and(|frame| frame > last_good) {
                *slot = None;
            }
        }
    }

    /// An IDR (asked for or the encoder's own, Gemini 8/3.2): the encoder
    /// dropped every long-term reference.
    pub fn key_frame(&mut self) {
        self.slots = [None; 2];
        self.next = 0;
        self.since = self.interval;
        self.forced = None;
    }

    /// The viewer acknowledged `frame_id` and every earlier picture of the
    /// stream (`ConfirmedReference::last_good`).
    pub fn confirm(&mut self, frame_id: u64) {
        self.confirmed = Some(self.confirmed.map_or(frame_id, |c| c.max(frame_id)));
    }

    /// The slot the next mark may take (see the type's text); `None` while
    /// the only other slot waits for its acknowledgement.
    fn target(&self) -> Option<usize> {
        let Some(confirmed) = self.confirmed else {
            return Some(self.next);
        };
        let kept = (0..2)
            .filter_map(|index| self.slots[index].map(|frame| (frame, index)))
            .filter(|(frame, _)| *frame <= confirmed)
            .max()
            .map(|(_, index)| index);
        (0..2).find(|index| {
            Some(*index) != kept && self.slots[*index].is_none_or(|frame| frame <= confirmed)
        })
    }

    /// Chooses the reference for the next picture; `false` when no marked
    /// picture is old enough.
    pub fn recover(&mut self, last_good: u64) -> bool {
        self.drop_after(last_good);
        let newest = (0..2)
            .filter_map(|index| self.slots[index].map(|frame| (frame, index)))
            .max();
        self.forced = newest.map(|(_, index)| index);
        self.forced.is_some()
    }
}

/// The host's newest picture the viewer can decode (see the module text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfirmedReference {
    /// The picture stream the host writes; acknowledgements of others do
    /// not count.
    gop: Option<u64>,
    last_good: Option<u64>,
}

impl ConfirmedReference {
    /// A new encoder chain (key frame, new encoder, decode error, another
    /// source): nothing of the old chain is a reference any more.
    pub fn restart(&mut self) {
        *self = Self::default();
    }

    /// The host opened picture stream `gop_id`; one that starts with a key
    /// frame begins a new chain, a continuation keeps the confirmed picture
    /// it references.
    pub fn opened(&mut self, gop_id: u64, continued: bool) {
        if !continued {
            self.last_good = None;
        }
        self.gop = Some(gop_id);
    }

    /// The host gave its stream up: late acknowledgements of it no longer
    /// count; the confirmed picture stays the reference of a continuation.
    pub fn stream_reset(&mut self) {
        self.gop = None;
    }

    /// `FRAME_ACK` for `frame_id` of `gop_id`.
    pub fn acked(&mut self, gop_id: u64, frame_id: u64) {
        if self.gop == Some(gop_id) {
            self.last_good = Some(self.last_good.map_or(frame_id, |last| last.max(frame_id)));
        }
    }

    /// The picture a continuation may reference.
    pub fn last_good(&self) -> Option<u64> {
        self.last_good
    }
}

#[cfg(test)]
#[path = "review1119_ltr_task_tests.rs"]
mod review1119_ltr_task_tests;
