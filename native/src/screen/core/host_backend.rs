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

use super::capture_admission::CaptureGrant;
use super::capture_image::CapturedImage;
use super::capture_records::CapturedRecords;
pub(crate) use super::capture_source_contract::{
    HostOutputPlan, PreparedSource, PreparedSourceInfo,
};
use super::input_model::{InputCapabilities, InputEvent};
use super::negotiate::{CodecChoice, CodecOffer};
use super::pixel_convert::{BgraImage, FrameSize, YuvMatrix};
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
    Frame(CapturedImage),
    /// Nothing changed since the last picture.
    Unchanged,
    /// The source exists but shows nothing usable for now (C1).
    Paused(SessionState),
}

/// Release-only endpoint belonging to one existing source.
pub(crate) trait ScreenInputSink: Send + Sync {
    fn legacy_input_invalid(&self) -> bool {
        false
    }
    fn inject(
        &self,
        _events: &[InputEvent],
        _reference_generation: u64,
        _presented: bool,
    ) -> Result<(), ControlBlock> {
        Err(ControlBlock::ControlUnavailable)
    }
    fn release(&self, events: &[InputEvent]) -> Result<(), ControlBlock>;
    fn family_snapshot(&self) -> io::Result<Option<crate::remote_window::FamilySnapshot>> {
        Ok(None)
    }
    fn family_operation(
        &self,
        _request: super::family_protocol::FamilyOperation,
        _active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> io::Result<super::family_protocol::FamilyOperationResult> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "source_has_no_window_family",
        ))
    }
    fn family_input(
        &self,
        _request: super::family_protocol::FamilyInput,
        _active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> io::Result<super::family_protocol::FamilyInputResult> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "source_has_no_window_family",
        ))
    }
}
/// One capture source; its SourceId determines whether it contains screen content.
pub(crate) trait ScreenSource: Send {
    fn legacy_input_invalid(&self) -> bool {
        false
    }
    /// Prepared capture implementations return their active bound grant.
    /// None supplies no allocation permission and is only a transitional
    /// state until every productive source uses prepare/activate.
    fn capture_grant(&self) -> Option<super::capture_admission::CaptureGrant> {
        None
    }
    /// A metadata-only wrapper may complete activation here. Some(state)
    /// preserves its pending state and requests no picture or encoder buffer.
    fn prepare_capture(
        &mut self,
        _request: &super::host_stage::StageRequest,
    ) -> io::Result<Option<SessionState>> {
        Ok(None)
    }
    /// Runs in the original capture worker after the admitted request waited
    /// in IPC. Platform sources may additionally verify their live geometry.
    fn inject_input(
        &mut self,
        backend: &dyn ScreenHostBackend,
        reference_generation: u64,
        _presented: bool,
        events: &[InputEvent],
    ) -> Result<(), ControlBlock> {
        if reference_generation != self.reference_generation() {
            return Err(ControlBlock::ControlUnavailable);
        }
        backend.inject(events)
    }
    /// Release-only endpoint retained across a source change or an in-flight capture.
    fn input_sink(&self) -> Option<std::sync::Arc<dyn ScreenInputSink>> {
        None
    }
    /// Size of the source's pictures; a change starts a new picture stream.
    fn size(&self) -> FrameSize;
    /// Proven capture allocation bound; unmodified adapters keep their old ceiling.
    fn frame_limits(&self) -> super::frame_budget::FrameLimits {
        super::frame_budget::FrameLimits::LEGACY
    }
    /// The allocation bound for this negotiated session; legacy adapters may
    /// retain a distinct surface-backed path without changing their v2 offer.
    fn frame_limits_for_contract(
        &self,
        contract: Option<super::frame_budget::FrameContract>,
    ) -> super::frame_budget::FrameLimits {
        contract.map_or_else(
            || self.frame_limits(),
            |contract| contract.limits.for_capture(self.frame_limits()),
        )
    }
    /// Matching raw/video pixels permit hybrid reconstruction; false retains the v2 size contract without hybrid operations.
    fn supports_frame_regions(&self) -> bool {
        true
    }
    /// Geometry/content identity beyond dimensions, such as a window-family revision.
    fn reference_generation(&self) -> u64 {
        0
    }
    /// Layout captured into the most recent successful family image.
    fn family_layout_revision(&self) -> Option<u64> {
        None
    }
    /// Stop background production while retaining the authorized source identity.
    fn set_capture_active(&mut self, _active: bool) -> io::Result<()> {
        Ok(())
    }
    /// The picture of encoded frame `frame_id` (the synthetic source draws
    /// the id into its barcode). With `keyframe` a source delivers its last
    /// picture even when nothing changed, so a key frame always has one.
    fn capture(&mut self, frame_id: u64, keyframe: bool) -> io::Result<SourceFrame>;
    /// A still image leaves the capture adapter only within the session's
    /// geometry. The raw source allocation retains its separate capture limit.
    fn capture_snapshot(
        &mut self,
        frame_id: u64,
        contract: Option<super::frame_budget::FrameContract>,
    ) -> io::Result<SourceFrame> {
        if let Some(contract) = contract {
            contract.validate()?;
        }
        let limits = self.frame_limits_for_contract(contract);
        match self.capture(frame_id, true)? {
            SourceFrame::Frame(image) => {
                let size = limits.fit(image.size());
                limits.check(size)?;
                Ok(SourceFrame::Frame(if size == image.size() {
                    image
                } else {
                    image.downscale(
                        size,
                        limits,
                        &image.grant(),
                        super::capture_admission::CaptureOwner::ScaledPixels,
                    )?
                }))
            }
            frame => Ok(frame),
        }
    }

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
/// Productive host output. Unlike the codec benchmark result, every record
/// carries the promise acquired before its own byte allocation.
pub(crate) struct CapturedEncodedFrame {
    pub records: CapturedRecords,
    pub keyframe: bool,
}

pub(crate) trait ScreenEncoder: Send {
    fn encode_captured(
        &mut self,
        _frame: &CapturedImage,
        _request: &super::host_stage::StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "encoder_has_no_capture_admission",
        ))
    }
    /// Encoder name for statistics (`tiles`, an MFT, `openh264`).
    fn name(&self) -> String;
    fn size(&self) -> FrameSize;
    /// A pending lossless correction needs one repeated capture after motion stops.
    fn needs_refresh(&self) -> bool {
        false
    }
    fn encode_frame(
        &mut self,
        frame: &BgraImage,
        request: &super::host_stage::StageRequest,
    ) -> io::Result<EncodedFrame> {
        self.encode(frame, request.keyframe, request.target_bps)
    }
    /// Encodes `frame` (of `size()`); no records when nothing changed.
    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame>;
}

/// How the chroma planes of a decoded picture are stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Chroma {
    /// One plane with interleaved U and V samples (`YuvPicture::uv`).
    Nv12,
    /// Separate planes of the same row pitch (`YuvPicture::uv_stride`);
    /// `YuvPicture::uv` stays empty.
    I420 { u: Vec<u8>, v: Vec<u8> },
}

/// The planes of a decoded picture in limited range, as the decoder made
/// them (SCR-M7, decision C4: the viewer converts them to RGB on the GPU).
/// Rows are `y_stride` and `uv_stride` bytes apart; the picture's size is
/// the one of its picture stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct YuvPicture {
    pub y: Vec<u8>,
    pub y_stride: usize,
    pub uv: Vec<u8>,
    pub uv_stride: usize,
    pub chroma: Chroma,
    pub matrix: YuvMatrix,
}

pub(crate) trait ScreenDecoder: Send {
    fn name(&self) -> String;
    /// Applies one record; `true` when an actual picture is available. A video
    /// decoder may return an older submitted picture; use `decoded_frame` for
    /// its identity, never the record most recently passed to this method.
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool>;
    /// Identity of the picture produced by the last `decode` that returned `true`.
    /// `None` is unproven output and must never receive a presentation receipt.
    fn decoded_frame(&self) -> Option<super::decoded_frame::DecodedFrame> {
        None
    }
    /// The last completed picture.
    fn picture(&self) -> &BgraImage;
    /// Asks to keep the decoded planes (`yuv_picture`) and to skip the
    /// conversion to BGRA; `picture()` then stays the last converted one
    /// (black if there never was one) and is not to be read. Only the H.264
    /// decoders honour it; the tile decoder always makes BGRA.
    fn prefer_yuv(&mut self, _on: bool) {}
    /// The planes of the last completed picture after `prefer_yuv(true)`.
    fn yuv_picture(&self) -> Option<YuvPicture> {
        None
    }
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
    fn memory_facts(&self) -> Option<super::frame_memory::MemoryFacts> {
        None
    }
    fn capacity_ceiling(
        &self,
        facts: Option<super::frame_memory::MemoryFacts>,
    ) -> super::frame_memory::CapacityCeiling {
        if facts.is_some() {
            super::frame_memory::CapacityCeiling::desktop(facts)
        } else {
            super::frame_memory::CapacityCeiling::bounded(self.session_memory_ceiling())
        }
    }
    /// Android's separately observed Java heap bound remains a session ceiling.
    fn session_memory_ceiling(&self) -> u64 {
        super::frame_budget::MAX_SESSION_WORKING_BYTES
    }
    fn icon(
        &self,
        _entry: super::app_icon::AuthorizedIconEntry<'_>,
    ) -> io::Result<Option<super::app_icon::IconReply>> {
        Ok(None)
    }
    /// A graphical session exists in which the screen can be shown.
    fn graphical_session(&self) -> bool;
    fn monitors(&self) -> Vec<MonitorInfo>;
    fn encoders(&self) -> Vec<CodecOffer>;
    fn input(&self) -> InputCapabilities;
    /// Platform limits for `HELLO_OK` (`consent_each_session`, …).
    fn limits(&self) -> Vec<String>;
    fn windows(&self) -> io::Result<WindowList>;
    /// Metadata-only source preparation. The additive default refuses this
    /// boundary; it must never open a legacy source without its capture grant.
    fn prepare_session_source(
        &self,
        _owner: &str,
        _session: &str,
        _source: &SourceId,
        _wanted: Option<FrameSize>,
        _control: bool,
        _admission: std::sync::Arc<super::capture_admission::SessionAdmission>,
    ) -> Result<Box<dyn PreparedSource>, SourceRefusal> {
        Err(SourceRefusal::new("capture_preparation_unavailable"))
    }
    /// Family metadata keeps the exact member identity and the same local or
    /// delegated admission. Activation still requires its bound member grant.
    fn prepare_family_member(
        &self,
        _owner: &str,
        _session: &str,
        _native: crate::remote_window::NativeWindowId,
        _admission: std::sync::Arc<super::capture_admission::SessionAdmission>,
    ) -> Result<Box<dyn PreparedSource>, SourceRefusal> {
        Err(SourceRefusal::new("family_capture_preparation_unavailable"))
    }
    fn family_windows(&self, _owner: &str) -> io::Result<super::family_protocol::FamilyListing> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "window_family_listing_unavailable",
        ))
    }
    /// Trusted host launch correlation only; publication remains ScreenView metadata.
    fn register_family_pin(
        &self,
        _owner: &str,
        _request_id: &str,
        _pin: crate::remote_window::WindowPin,
    ) -> io::Result<super::family_protocol::FamilyListEntry> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "verified_family_registration_unavailable",
        ))
    }
    fn open_session_source(
        &self,
        _owner: &str,
        _session: &str,
        source: &SourceId,
        wanted: Option<FrameSize>,
        _control: bool,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        if super::family_protocol::selector(source).is_some() {
            return Err(SourceRefusal::new("window_family_unavailable"));
        }
        self.open_source(source, wanted)
    }
    /// Opens `source`; `wanted` is the size a viewer asked for, used by
    /// sources without a size of their own (`synthetic`).
    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal>;
    /// Opens only this member's own surface. A desktop/root crop is forbidden,
    /// including when the independent capture API stops working after opening.
    fn open_family_member(
        &self,
        _native: crate::remote_window::NativeWindowId,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        Err(SourceRefusal::new(
            "window_family_member_capture_unsupported",
        ))
    }
    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>>;
    /// Carries the negotiated image and codec-rate contract to in-process encoders.
    fn open_encoder_for_stage(
        &self,
        request: &super::host_stage::StageRequest,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.open_encoder(&request.choice, request.size)
    }
    fn open_encoder_for_stage_admitted(
        &self,
        _request: &super::host_stage::StageRequest,
        _grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "encoder_has_no_capture_admission",
        ))
    }
    /// Injects the input of the controlling viewer.
    fn inject(&self, events: &[InputEvent]) -> Result<(), ControlBlock>;
    /// Family coordinates are relative to the exact admitted capture geometry.
    fn inject_family(
        &self,
        _member: &crate::remote_window::CaptureMember,
        events: &[InputEvent],
    ) -> Result<(), ControlBlock> {
        self.inject(events)
    }
    /// Release previously injected controls even after their window loses focus.
    fn release_input(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        self.inject(events)
    }
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
    /// Opens a request for `capability`; the ticket answers at once when
    /// nobody can be asked.
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

#[path = "host_backend_codecs.rs"]
mod tile_adapters;
pub(crate) use tile_adapters::{TilesScreenDecoder, TilesScreenEncoder};
