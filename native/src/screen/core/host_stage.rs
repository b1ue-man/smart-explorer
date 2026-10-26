//! Capture and encoding in the host process (SCR-M3): a source opened there
//! hands the pipeline this stage instead of raw pictures
//! (`ScreenSource::host_stage`). The pipeline asks it for one encoded
//! picture per frame; the stage opens its encoder anew whenever
//! `encoder_epoch` changes (a new picture group, another size or codec) and
//! reports the source size with every answer, so a changed screen size
//! reaches the quality ladder. A crash of the host process ends only the
//! session: the error carries the code the viewer gets.
use std::io;

use super::negotiate::CodecChoice;
use super::pixel_convert::FrameSize;
use super::session_state::SessionState;
use super::wire_hello::ScreenErrorCode;

/// What the pipeline asks a stage for one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StageRequest {
    pub frame_id: u64,
    /// Present only after both endpoints confirmed the v2 contract.
    pub frame_contract: Option<super::frame_budget::FrameContract>,
    /// Encoder-input coordinates; None means unverified geometry and transfers full deltas.
    pub visible_rects: Option<Vec<super::visibility::VisibleRect>>,
    pub keyframe: bool,
    pub target_bps: u64,
    /// Size of the picture group, the encoder's input.
    pub size: FrameSize,
    pub choice: CodecChoice,
    /// Changes whenever the pipeline starts a new encoder.
    pub encoder_epoch: u64,
    /// Frame rate the encoder's rate control assumes.
    pub fps: u32,
}

/// The encoder that produced a picture, for `STATE` and the statistics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StageEncoder {
    pub name: String,
    pub hardware: bool,
}

/// What a stage delivered for one frame; `source_size` is the size of the
/// source at capture time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StageFrame {
    Encoded {
        records: Vec<Vec<u8>>,
        keyframe: bool,
        capture_ts_us: u64,
        encoder: StageEncoder,
        source_size: FrameSize,
    },
    /// Nothing changed, or the source size changed and the request no
    /// longer fits it.
    Unchanged {
        source_size: FrameSize,
    },
    Paused(SessionState),
}

/// A failure of the stage; `code` goes to the viewer in `ERROR`.
#[derive(Debug)]
pub(crate) struct StageError {
    pub code: ScreenErrorCode,
    pub error: io::Error,
}

impl StageError {
    pub(crate) fn new(code: ScreenErrorCode, error: io::Error) -> Self {
        Self { code, error }
    }
}

/// A source whose capture and encoding run in the host process.
pub(crate) trait HostStage: Send {
    /// Captures, scales and encodes one frame as `request` says.
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError>;
}
