//! Identity of an actual decoder output, independent of the latest input call.
use super::wire::invalid;
use super::wire_binary::FrameRecord;
use std::collections::BTreeMap;
use std::fmt;
use std::io;

/// H.264 Annex A limits the DPB to 16 frames; allow the current input as well.
/// ITU-T H.264 (08/2024), A.3.1: max_dec_frame_buffering <= MaxDpbFrames <= 16.
/// This also bounds metadata retained when a decoder never produces output.
pub(crate) const MAX_PENDING_FRAMES: usize = 17;

/// Distinct from corrupt bitstream errors: hybrid can switch to exact tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecoderFallback {
    NeedsMoreInput,
    OutputIdentityUnavailable,
}
impl fmt::Display for DecoderFallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DecoderFallback {}
impl DecoderFallback {
    pub(crate) fn error(self) -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, self)
    }
    pub(crate) fn from_error(error: &io::Error) -> Option<Self> {
        error.get_ref()?.downcast_ref::<Self>().copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DecodedFrame {
    pub frame_id: u64,
    pub capture_ts_us: u64,
    pub keyframe: bool,
}
impl DecodedFrame {
    pub(crate) fn from_record(record: &FrameRecord) -> Self {
        Self {
            frame_id: record.frame_id,
            capture_ts_us: record.capture_ts_us,
            keyframe: record.is_keyframe(),
        }
    }
}

/// Codec-local sample token to submitted picture identity. No pixel storage.
#[derive(Default)]
pub(crate) struct FrameIdentities(BTreeMap<u64, DecodedFrame>);
impl FrameIdentities {
    pub(crate) fn insert(&mut self, token: u64, frame: DecodedFrame) -> io::Result<()> {
        if self.0.len() >= MAX_PENDING_FRAMES || self.0.contains_key(&token) {
            return Err(invalid("decoder identity window exhausted or repeated"));
        }
        self.0.insert(token, frame);
        Ok(())
    }
    pub(crate) fn take(&mut self, token: u64) -> io::Result<DecodedFrame> {
        let frame = self
            .0
            .remove(&token)
            .ok_or_else(|| DecoderFallback::OutputIdentityUnavailable.error())?;
        // Outputs older than the published picture can no longer be presented.
        self.0.retain(|pending, _| *pending > token);
        Ok(frame)
    }
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}
