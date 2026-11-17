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
use super::capture_picture::{Picture, SourcePicture};
use super::capture_records::CapturedRecords;
pub use super::capture_source_contract::{HostOutputPlan, PreparedSource, PreparedSourceInfo};
use super::host_stage::StageRequest;
use super::input_model::{InputCapabilities, InputEvent};
use super::negotiate::{CodecChoice, CodecOffer};
use super::pixel_convert::{BgraImage, FrameSize, YuvMatrix};
use super::session_state::SessionState;
use super::tiles_codec::{TileDecoder, TileEncoder, MAX_TILES_RECORD_BYTES};
use super::window_list::{MonitorInfo, SourceId, WindowList};
use super::wire_binary::FrameRecord;
use super::wire_control::ControlBlock;
use super::wire_hello::HostPlatform;

/// What one capture delivered.
pub enum SourceFrame {
    Frame(CapturedImage),
    /// Nothing changed since the last picture.
    Unchanged,
    /// The source exists but shows nothing usable for now (C1).
    Paused(SessionState),
}

/// Release-only endpoint belonging to one existing source.
pub trait ScreenInputSink: Send + Sync {
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
    /// Takes the pointer out of the picture (`true`) or back into it (P6,
    /// `cursor_overlay.rs`); `false` while this source does not separate it
    /// (it cannot, or its capture has not started: the wish is kept).
    /// Blocks while the source answers: call it off the async runtime.
    fn set_cursor_overlay(&self, _on: bool) -> bool {
        false
    }
    /// Where the separated pointer arrives, for a source that can.
    fn cursor_feed(&self) -> Option<std::sync::Arc<super::cursor_overlay::CursorFeed>> {
        None
    }
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
pub trait ScreenSource: Send {
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
    /// Like `capture`, but the picture may stay on the GPU (BK3b); only the
    /// host process's picture stream takes such pictures.
    fn capture_picture(&mut self, frame_id: u64, keyframe: bool) -> io::Result<SourcePicture> {
        self.capture(frame_id, keyframe).map(SourcePicture::from)
    }
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
pub struct EncodedFrame {
    pub records: Vec<Vec<u8>>,
    pub keyframe: bool,
}
/// Productive host output. Unlike the codec benchmark result, every record
/// carries the promise acquired before its own byte allocation.
#[derive(Debug)]
pub struct CapturedEncodedFrame {
    pub records: CapturedRecords,
    pub keyframe: bool,
}
type Encoded = CapturedEncodedFrame;

pub trait ScreenEncoder: Send {
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
    /// A picture that may lie on the GPU; an encoder that takes no GPU
    /// pictures gets its CPU pixels through the readback (BK3b).
    fn encode_picture(&mut self, picture: &Picture, request: &StageRequest) -> io::Result<Encoded> {
        super::capture_picture::encode_on_cpu(self, picture, request)
    }
    /// Encoder name for statistics (`tiles`, an MFT, `openh264`).
    fn name(&self) -> String;
    fn size(&self) -> FrameSize;
    /// A pending lossless correction needs one repeated capture after motion stops.
    fn needs_refresh(&self) -> bool {
        false
    }
    /// B-36: after a stream reset the next picture may reference only
    /// `last_good` (a frame id the viewer confirmed) or older; an encoder
    /// without reference control needs a key frame, as before.
    fn recover_references(&mut self, _last_good: u64) -> super::ref_recovery::Recovery {
        super::ref_recovery::Recovery::KeyframeNeeded
    }
    /// What the encoder made of the last rate it was given (plan
    /// 2026-11-17, `rate_control::honour`); `None`: it does not tell.
    fn rate_state(&self) -> Option<super::rate_control::honour::EncoderRateState> {
        None
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

pub trait ScreenDecoder: Send {
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
    /// Hands the planes of the last completed picture over instead of
    /// copying them (plan „Bildkette“ BK9 U1, V1); afterwards
    /// `yuv_picture` is unspecified until the next picture. The default
    /// copies (`yuv_picture`).
    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.yuv_picture()
    }
    /// Hands the last completed BGRA picture over; the default copies it.
    /// A decoder whose picture is its own reference (tiles) copies into a
    /// returned buffer.
    fn take_picture(&mut self) -> BgraImage {
        self.picture().clone()
    }
    /// Gives the storage of a shown picture back, so the next one needs no
    /// new allocation while size and format stay; the default drops it.
    fn recycle_yuv(&mut self, _planes: YuvPicture) {}
    fn recycle_picture(&mut self, _image: BgraImage) {}
    /// P8 (2026-11-17): a presenter on the decoder's device takes pictures as
    /// GPU surfaces; only FFmpeg's D3D11VA stage honours it.
    fn prefer_gpu(&mut self, _on: bool) {}
    /// The last picture as a GPU surface after `prefer_gpu(true)`, once.
    fn take_gpu_picture(&mut self) -> Option<super::gpu_picture::GpuPicture> {
        None
    }
    /// B-36: whether this decoder may go on over a continuation
    /// (`FLAG_CONTINUE`) and decode it correctly from its kept references;
    /// `false` makes the viewer ask for a key frame instead (`gop_continue`).
    fn keeps_references(&self) -> bool {
        false
    }
}

/// The screen host of one platform.
pub trait ScreenHostBackend: Send + Sync {
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
    /// Opens the next admitted stage encoder not in `refused` (W4, B3).
    fn open_stage_encoder(
        &self,
        request: &super::host_stage::StageRequest,
        refused: &[String],
        grant: CaptureGrant,
    ) -> io::Result<super::codec_registry::OpenedEncoder> {
        let encoder = self.open_encoder_for_stage_admitted(request, grant)?;
        let factory = encoder.name();
        if request.choice.codec != super::negotiate::CodecKind::Tiles
            && refused.iter().any(|name| name == &factory)
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("encoder {factory} was refused in this session"),
            ));
        }
        Ok(super::codec_registry::OpenedEncoder {
            encoder,
            factory,
            hardware: request.choice.encoder_hardware,
        })
    }
    /// Number of rank stages available for `request` (W4, B3).
    fn encoder_stages(&self, _request: &super::host_stage::StageRequest) -> usize {
        1
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
    /// The virtual display in the viewer's resolution (plan 2026-11-17 P5);
    /// `None`: this host never changes its display (the old behaviour).
    fn virtual_display(
        &self,
    ) -> Option<std::sync::Arc<dyn super::virtual_display_host::VirtualDisplayDriver>> {
        None
    }
}

#[path = "host_backend_consent.rs"]
mod consent;
pub use consent::{ConsentBroker, SessionIndicator};

#[path = "host_backend_codecs.rs"]
mod tile_adapters;
pub use tile_adapters::{
    Chroma, SourceRefusal, TilesScreenDecoder, TilesScreenEncoder, YuvPicture,
};
