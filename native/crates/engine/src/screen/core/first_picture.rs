//! The first picture of a video encoder (plan 2026-11-18 „Netzrate“; user
//! report: with AV1 chosen the picture stayed black from the start).
//!
//! Every new picture group starts with a key frame the pipeline asks for;
//! the capture delivers its last picture again for it, also on a still
//! screen (`ScreenSource::capture` with `keyframe`; DXGI `again`). An
//! encoder that got that picture but delivered nothing (no records, or no
//! key frame) left the session black without a fallback. The watch counts
//! only such answers (`StageAnswer::blames_encoder`); a capture that had
//! nothing for the encoder („unchanged“) is no failure of the encoder. After
//! `FIRST_PICTURE_DEADLINE_US` and at least `FIRST_PICTURE_MIN_SILENT`
//! answers the encoder counts as failed and the fallback order applies
//! (another encoder of the same codec; another codec only under
//! „Automatisch“).
use super::frame_size_caps::video_keyframe;
use super::negotiate::{CodecChoice, CodecKind};

/// Whether an encoded picture of `codec` starts a picture group: its flag,
/// or for a picture of one record its bitstream (measurement of 2026-11-18,
/// AV1 on an AMD host: a key frame is told by the stream as well, AV1 by the
/// frame header's `frame_type` KEY_FRAME of a shown frame, H.264/HEVC by an
/// IDR unit, `video_keyframe`). A picture of several records (hybrid
/// regions) keeps its flag alone: its other records are no video stream.
pub fn starts_group<R: AsRef<[u8]>>(
    codec: CodecKind,
    flagged: bool,
    records: impl IntoIterator<Item = R>,
) -> bool {
    if flagged {
        return true;
    }
    if !codec.is_video() {
        return false;
    }
    let mut records = records.into_iter();
    match (records.next(), records.next()) {
        (Some(only), None) => video_keyframe(codec, only.as_ref()),
        _ => false,
    }
}

/// What a stage answered to a key-frame request without a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageAnswer {
    /// The capture gave the encoder nothing (still screen, a capture that
    /// waits): not the encoder's failure.
    NothingCaptured,
    /// The encoder got a picture and returned no records.
    EncodedNothing,
    /// The encoder returned a picture that starts no group (no key frame).
    NoKeyFrame,
}

impl StageAnswer {
    /// Whether the answer counts against the encoder.
    pub fn blames_encoder(self) -> bool {
        self != Self::NothingCaptured
    }
}

/// How long a video encoder may stay without a first picture.
pub const FIRST_PICTURE_DEADLINE_US: u64 = 4_000_000;
/// Silent answers needed besides the time (one slow answer is no failure).
pub const FIRST_PICTURE_MIN_SILENT: u32 = 3;

/// Silent answers of the stage for the pairing `watched`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FirstPictureWatch {
    watched: Option<CodecChoice>,
    since_us: u64,
    silent: u32,
}

impl FirstPictureWatch {
    /// The stage answered a key-frame request for `choice` without a
    /// picture at `now_us`; `true` when the encoder counts as failed.
    pub fn on_silent(&mut self, choice: CodecChoice, now_us: u64) -> bool {
        if self.watched != Some(choice) {
            *self = Self {
                watched: Some(choice),
                since_us: now_us,
                silent: 0,
            };
        }
        self.silent = self.silent.saturating_add(1);
        self.silent >= FIRST_PICTURE_MIN_SILENT
            && now_us.saturating_sub(self.since_us) >= FIRST_PICTURE_DEADLINE_US
    }

    /// A picture was sent, the source paused or the encoder was given up:
    /// the watch starts again with the next silent answer.
    pub fn restart(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
#[path = "rollout_first_picture_task_tests.rs"]
mod rollout_first_picture_task_tests;
