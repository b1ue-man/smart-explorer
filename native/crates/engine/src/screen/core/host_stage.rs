//! Capture and encoding in the host process (SCR-M3): a source opened there
//! hands the pipeline this stage instead of raw pictures
//! (`ScreenSource::host_stage`). The pipeline asks it for one encoded
//! picture per frame; the stage opens its encoder anew whenever
//! `encoder_epoch` changes (a new picture group, another size or codec) and
//! reports the source size with every answer, so a changed screen size
//! reaches the quality ladder. A crash of the host process ends only the
//! session: the error carries the code the viewer gets.
//!
//! A stage that paces capture and encoding itself (plan 2026-11-14
//! „Bildkette“ B-27, the host process of `os::host_worker_client`) gets the
//! session's cadence and whether one picture may be encoded ahead of its
//! frame (`StreamHint`, decided from the pacer's queue) before each
//! `produce`, and reports afterwards what the process measured for the
//! picture it delivered (`StreamReport`).
use std::io;

use super::negotiate::CodecChoice;
use super::pixel_convert::FrameSize;
use super::session_state::SessionState;
use super::wire_hello::ScreenErrorCode;

/// What the pipeline asks a stage for one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRequest {
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
pub struct StageEncoder {
    pub name: String,
    pub hardware: bool,
}

/// What a stage delivered for one frame; `source_size` is the size of the
/// source at capture time.
#[derive(Debug, PartialEq, Eq)]
pub enum StageFrame {
    Encoded {
        records: super::capture_records::CapturedRecords,
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
pub struct StageError {
    pub code: ScreenErrorCode,
    pub error: io::Error,
}

impl StageError {
    pub fn new(code: ScreenErrorCode, error: io::Error) -> Self {
        Self { code, error }
    }
}

/// What the pipeline tells a self-paced stage before a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamHint {
    /// Frames per second of the session's cadence; 0 means `request.fps`.
    pub cadence_fps: u32,
    /// One more picture may be captured and encoded before its frame is due:
    /// the pacer's queue holds room for this frame and the next one.
    pub ahead: bool,
    /// Plan 2026-11-17 P9: the constant quality of sharp mode
    /// (`rate_model::SHARP_TARGET_QUALITY`); `None` for a constant bit rate.
    pub quality_target: Option<u8>,
}

/// What a self-paced stage measured for the answer of its last `produce`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamReport {
    /// The frame id the picture was captured with (the test picture draws it).
    pub frame_id: Option<u64>,
    /// Duration of the capture call of the picture.
    pub capture_us: u64,
    /// Duration of scaling, conversion and encoding of the picture.
    pub encode_us: u64,
    /// Pictures captured and dropped before encoding since the last report
    /// (a newer capture replaced them).
    pub dropped_before_encode: u64,
}

/// A source whose capture and encoding run in the host process.
pub trait HostStage: Send {
    /// Captures, scales and encodes one frame as `request` says.
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError>;
    /// Whether the stage paces capture and encoding itself (B-27): the
    /// pipeline then asks for the next picture as soon as the previous one
    /// is written (or an acknowledgement frees the queue), not per tick of
    /// the session loop (plan „Bildkette“ BK3b).
    fn self_paced(&self) -> bool {
        false
    }
    /// The cadence and the credit ahead for the next `produce`; a stage that
    /// captures on request ignores it.
    fn set_stream_hint(&mut self, _hint: StreamHint) {}
    /// What the last `produce` measured in the host process, once.
    fn take_stream_report(&mut self) -> Option<StreamReport> {
        None
    }
    /// A notice for the viewer that ends nothing (review 2026-11-19): what
    /// the stage could not do as asked while its encoder keeps running, for
    /// example a device minimum above the user's „Höchste Datenrate“ or a
    /// parameter the encoder refused. A stage gives each notice once, when
    /// it arises (every notice is a `STATE` frame). Taken after `produce`; the
    /// pipeline sends it as `encoder_notice: <text>` in the next `STATE`
    /// reason (`Pipeline::note_encoder`). One short English sentence; the
    /// viewers show it after „Hinweis vom Gerät:“.
    fn take_notice(&mut self) -> Option<String> {
        None
    }
    /// Hands the station times and capture facts measured in the host
    /// process since the last call to the session's meter (plan 2026-11-14
    /// „Bildkette“ BK0); a stage without a host process has none.
    fn drain_chain(&mut self, _meter: &mut super::chain_meter::ChainMeter) {}
    /// Review 2026-11-19 (X3): the notice of an encoder that runs at a lowest
    /// bit rate above the viewer's „Höchste Datenrate“ `ceiling_bps`
    /// (`floor_notice`), once per lowest rate and ceiling; the stream's byte
    /// budget keeps the ceiling. A stage whose host process reports no such
    /// rate has none (the Android bridge names its own in `take_notice`).
    fn floor_notice(&mut self, _ceiling_bps: Option<u64>) -> Option<String> {
        None
    }
    /// B-36: whether the stage passes `recover_references` on to its
    /// encoder (`ScreenEncoder::recover_references`). A stage that does not
    /// keeps the old reset: a new encoder and a key frame.
    fn continues_references(&self) -> bool {
        false
    }
    /// B-36: the next picture continues a reset stream and may reference
    /// only `last_good` or older; the encoder answers with a key frame when
    /// it cannot. Only called when `continues_references` holds.
    fn recover_references(&mut self, _last_good: u64) {}
}

/// Review 2026-11-19 (X3): the sentence of an encoder whose lowest bit rate
/// `floor_bps` lies above the viewer's `ceiling_bps` (the wording
/// `codec_reason_text::encoder_notice_text` shows in German on every
/// viewer); `None` while it does not, or without a ceiling.
pub fn floor_above_ceiling(floor_bps: u64, ceiling_bps: Option<u64>) -> Option<String> {
    let ceiling = ceiling_bps.filter(|ceiling| floor_bps > *ceiling)?;
    Some(format!(
        "the encoder's lowest bit rate {floor_bps} bit/s is above the highest data rate \
         {ceiling} bit/s; fewer pictures keep the ceiling"
    ))
}
