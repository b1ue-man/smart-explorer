//! What the screen host needs from its platform (SCR-M2): a backend that
//! names the platform, its monitors, encoders, input and limits and opens
//! sources and encoders; a source that captures pictures; an encoder and a
//! decoder per codec; the indicator that shows the host user that the
//! screen is watched (C7); and the consent broker through which the host
//! asks its user on „Fragen“ (contract V7: S1 `access_approval` implements
//! it). The lossless tile codec is platform-neutral, so its encoder and
//! decoder adapters live here; capture and H.264 follow under `os/` (SCR-M3
//! to SCR-M6, SCR-M8).
use std::io;

use super::input_model::{InputCapabilities, InputEvent};
use super::negotiate::{CodecChoice, CodecOffer};
use super::pixel_convert::{BgraImage, FrameSize};
use super::session_state::SessionState;
use super::tiles_codec::{TileDecoder, TileEncoder, MAX_TILES_RECORD_BYTES};
use super::window_list::{MonitorInfo, SourceId, WindowList};
use super::wire_binary::FrameRecord;
use super::wire_control::ControlBlock;
use super::wire_hello::HostPlatform;
use crate::share::access_approval::{ApprovalBroker, ApprovalSummary, ApprovalTicket};
use crate::share::access_grant::Capability;
use crate::share::relation_proof::ProvenPrincipal;

/// What one capture delivered.
pub(crate) enum SourceFrame {
    Frame(BgraImage),
    /// Nothing changed since the last picture.
    Unchanged,
    /// The source exists but shows nothing usable for now (C1).
    Paused(SessionState),
}

/// One capture source of the host (`monitor:<id>`, `window:<id>`,
/// `synthetic`); whether it shows screen content says its `SourceId`.
pub(crate) trait ScreenSource: Send {
    /// Size of the source's pictures; a change starts a new picture stream.
    fn size(&self) -> FrameSize;
    /// The picture of encoded frame `frame_id` (the synthetic source draws
    /// the id into its barcode). With `keyframe` a source delivers its last
    /// picture even when nothing changed, so a key frame always has one.
    fn capture(&mut self, frame_id: u64, keyframe: bool) -> io::Result<SourceFrame>;
    /// The stage of a source whose capture and encoding run in the host
    /// process (SCR-M3, `os::host_worker_client`); `None` keeps both here.
    fn host_stage(&mut self) -> Option<&mut dyn super::host_stage::HostStage> {
        None
    }
}

/// The records of one encoded picture.
pub(crate) struct EncodedFrame {
    pub records: Vec<Vec<u8>>,
    pub keyframe: bool,
}

pub(crate) trait ScreenEncoder: Send {
    /// Encoder name for statistics (`tiles`, an MFT, `openh264`).
    fn name(&self) -> String;
    fn size(&self) -> FrameSize;
    /// Encodes `frame` (of `size()`); no records when nothing changed.
    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame>;
}

pub(crate) trait ScreenDecoder: Send {
    fn name(&self) -> String;
    /// Applies one record; `true` when it completed a picture.
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool>;
    /// The last completed picture.
    fn picture(&self) -> &BgraImage;
}

/// Why a source cannot be opened; the key is the reason of `no_display`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceRefusal {
    pub reason: String,
}

impl SourceRefusal {
    pub(crate) fn new(reason: &str) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// The screen host of one platform.
pub(crate) trait ScreenHostBackend: Send + Sync {
    fn platform(&self) -> HostPlatform;
    /// A graphical session exists in which the screen can be shown.
    fn graphical_session(&self) -> bool;
    fn monitors(&self) -> Vec<MonitorInfo>;
    fn encoders(&self) -> Vec<CodecOffer>;
    fn input(&self) -> InputCapabilities;
    /// Platform limits for `HELLO_OK` (`consent_each_session`, …).
    fn limits(&self) -> Vec<String>;
    fn windows(&self) -> io::Result<WindowList>;
    /// Opens `source`; `wanted` is the size a viewer asked for, used by
    /// sources without a size of their own (`synthetic`).
    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal>;
    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>>;
    /// Injects the input of the controlling viewer.
    fn inject(&self, events: &[InputEvent]) -> Result<(), ControlBlock>;
}

/// The host user's sign that the screen is watched (C7): the window
/// `smart_explorer --screen-indicator` on desktops.
pub(crate) trait SessionIndicator: Send + Sync {
    /// Whether the indicator can be shown at all (desktop app present).
    fn available(&self) -> bool;
    /// Shows it, or keeps it shown; an error ends the session with
    /// `indicator_unavailable`.
    fn show(&self) -> Result<(), String>;
}

/// The screen strand's view of the S1 confirmation service (V7).
pub(crate) trait ConsentBroker: Send + Sync {
    /// Opens a request for `capability`; the ticket answers at once when no
    /// reservation is free or nobody can be asked.
    fn request(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
    ) -> ApprovalTicket;
}

impl ConsentBroker for ApprovalBroker {
    fn request(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
    ) -> ApprovalTicket {
        ApprovalBroker::request(self, principal, capability, summary)
    }
}

/// The lossless tile codec as a `ScreenEncoder`.
pub(crate) struct TilesScreenEncoder {
    encoder: TileEncoder,
}

impl TilesScreenEncoder {
    pub(crate) fn new(size: FrameSize) -> io::Result<Self> {
        Ok(Self {
            encoder: TileEncoder::new(size)?,
        })
    }
}

impl ScreenEncoder for TilesScreenEncoder {
    fn name(&self) -> String {
        "tiles".into()
    }

    fn size(&self) -> FrameSize {
        self.encoder.size()
    }

    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        _target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        let records = self
            .encoder
            .encode(&frame.view(), keyframe, MAX_TILES_RECORD_BYTES)?;
        Ok(EncodedFrame { records, keyframe })
    }
}

/// The lossless tile codec as a `ScreenDecoder`.
pub(crate) struct TilesScreenDecoder {
    decoder: TileDecoder,
}

impl TilesScreenDecoder {
    pub(crate) fn new(size: FrameSize) -> io::Result<Self> {
        Ok(Self {
            decoder: TileDecoder::new(size)?,
        })
    }
}

impl ScreenDecoder for TilesScreenDecoder {
    fn name(&self) -> String {
        "tiles".into()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoder.apply_record(&record.payload)?;
        Ok(!record.has_more())
    }

    fn picture(&self) -> &BgraImage {
        self.decoder.image()
    }
}
