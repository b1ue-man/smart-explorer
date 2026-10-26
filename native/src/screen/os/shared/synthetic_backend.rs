//! The screen host of SCR-M2 on every platform: the test picture
//! `synthetic` (C9) with the lossless tile codec, no screen capture and no
//! input yet. Capture, H.264 and input of Windows, Linux and Android follow
//! with SCR-M3 to SCR-M8 as backends of their own. A session that asks for a
//! monitor or window gets `no_display` with the reason `capture_unavailable`;
//! the host names that in its limits (`test_picture_only`).
use std::io;

use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SessionAdmission, SourceKey,
};
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_plan::CapturePhases;
use crate::screen::core::capture_source_contract::{
    HostOutputPlan, PreparedSource, PreparedSourceInfo,
};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceFrame, SourceRefusal, TilesScreenEncoder,
};
use crate::screen::core::host_codec_adapters::HostInputPixels;
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::synthetic::{render, MIN_SYNTHETIC_SIZE};
use crate::screen::core::window_list::{ListSource, MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::HostPlatform;
use std::sync::Arc;

/// Size of the test picture when the viewer names none.
pub(crate) const DEFAULT_SYNTHETIC_SIZE: FrameSize = FrameSize::new(1280, 720);
/// Limit of a host that can show only the test picture.
pub(crate) const LIMIT_TEST_PICTURE_ONLY: &str = "test_picture_only";
/// Reason of `no_display` for a monitor or window on such a host.
pub(crate) const CAPTURE_UNAVAILABLE: &str = "capture_unavailable";

/// Whether this process runs in a graphical session in which a screen can
/// be shown: Windows (the worker runs in its user's session), `DISPLAY` or
/// `WAYLAND_DISPLAY` elsewhere. Android hosts follow with SCR-M8.
pub(crate) fn graphical_session() -> bool {
    !cfg!(target_os = "android") && crate::share::consent_launch::SessionFacts::from_env().graphical
}

fn platform() -> HostPlatform {
    if cfg!(windows) {
        HostPlatform::Windows
    } else if cfg!(target_os = "android") {
        HostPlatform::Android
    } else {
        HostPlatform::Linux
    }
}

/// The host backend of SCR-M2.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SyntheticBackend;

impl ScreenHostBackend for SyntheticBackend {
    fn capacity_ceiling(
        &self,
        facts: Option<crate::screen::core::frame_memory::MemoryFacts>,
    ) -> crate::screen::core::frame_memory::CapacityCeiling {
        super::session_memory::capacity_ceiling(facts)
    }
    fn prepare_session_source(
        &self,
        _owner: &str,
        session: &str,
        source: &SourceId,
        wanted: Option<FrameSize>,
        _control: bool,
        admission: Arc<SessionAdmission>,
    ) -> Result<Box<dyn PreparedSource>, SourceRefusal> {
        if admission.session() != session {
            return Err(SourceRefusal::new("synthetic_session_mismatch"));
        }
        if *source != SourceId::Synthetic {
            return Err(SourceRefusal::new(CAPTURE_UNAVAILABLE));
        }
        Ok(Box::new(PreparedSynthetic {
            key: SourceKey::new(session, "synthetic")
                .map_err(|error| SourceRefusal::new(&error.to_string()))?,
            size: synthetic_size(wanted),
        }))
    }
    fn open_encoder_for_stage_admitted(
        &self,
        request: &crate::screen::core::host_stage::StageRequest,
        grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        if request.choice.codec != CodecKind::Tiles {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "synthetic_has_no_h264",
            ));
        }
        let limits = request
            .frame_contract
            .map_or(FrameLimits::LEGACY, |contract| contract.limits);
        let encoder = Box::new(
            crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(
                request.size,
                limits,
                grant.clone(),
            )?,
        );
        crate::screen::core::host_hybrid::wrap_captured(encoder, request, grant)
    }
    fn memory_facts(&self) -> Option<crate::screen::core::frame_memory::MemoryFacts> {
        super::session_memory::facts()
    }
    fn platform(&self) -> HostPlatform {
        platform()
    }

    fn graphical_session(&self) -> bool {
        graphical_session()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        Vec::new()
    }

    fn encoders(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::tiles()]
    }

    fn input(&self) -> InputCapabilities {
        InputCapabilities::default()
    }

    fn limits(&self) -> Vec<String> {
        vec![LIMIT_TEST_PICTURE_ONLY.into()]
    }

    fn windows(&self) -> io::Result<WindowList> {
        Ok(WindowList {
            windows: Vec::new(),
            apps: Vec::new(),
            list_source: ListSource::Unknown,
            limits: vec![LIMIT_TEST_PICTURE_ONLY.into()],
        })
    }

    fn open_source(
        &self,
        source: &SourceId,
        _wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        match source {
            SourceId::Synthetic => Err(SourceRefusal::new(
                "synthetic_capture_requires_prepare_activate",
            )),
            SourceId::Monitor(_) | SourceId::Window(_) => {
                Err(SourceRefusal::new(CAPTURE_UNAVAILABLE))
            }
        }
    }

    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        match choice.codec {
            CodecKind::Tiles => Ok(Box::new(TilesScreenEncoder::new(size)?)),
            CodecKind::H264 => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this host has no H.264 encoder yet",
            )),
        }
    }

    fn inject(&self, _events: &[InputEvent]) -> Result<(), ControlBlock> {
        Err(ControlBlock::UnsupportedOnHost)
    }
}

/// The test picture as a source: a new picture for every frame id.
pub(crate) struct SyntheticSource {
    image: Option<CapturedImage>,
    size: FrameSize,
    grant: CaptureGrant,
}
fn synthetic_size(wanted: Option<FrameSize>) -> FrameSize {
    wanted
        .filter(|size| {
            size.is_valid_frame()
                && size.width >= MIN_SYNTHETIC_SIZE.width
                && size.height >= MIN_SYNTHETIC_SIZE.height
        })
        .unwrap_or(DEFAULT_SYNTHETIC_SIZE)
}
struct PreparedSynthetic {
    key: SourceKey,
    size: FrameSize,
}
impl PreparedSource for PreparedSynthetic {
    fn source_key(&self) -> &SourceKey {
        &self.key
    }
    fn description(&self) -> PreparedSourceInfo {
        PreparedSourceInfo {
            native_size: Some(self.size),
            capture_limits: FrameLimits::LEGACY,
            supports_regions: true,
            state: crate::screen::core::session_state::SessionState::Starting,
            generation: CaptureGeneration::new(1),
        }
    }
    fn capture_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        let mut plan = CapturePlan::new(self.key.clone(), CaptureGeneration::new(1));
        plan.add_bgra(CaptureOwner::SourcePixels, self.size, 2, CapturePhases::ALL)?;
        output.append_to(
            &mut plan,
            if output.size() == self.size {
                HostInputPixels::BorrowedSource
            } else {
                HostInputPixels::SeparateImage
            },
        )?;
        Ok(Some(plan))
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        self.description()
            .check_grant(&self.key, &grant)
            .map_err(|error| SourceRefusal::new(&error.to_string()))?;
        Ok(Box::new(SyntheticSource {
            image: None,
            size: self.size,
            grant,
        }))
    }
}

impl ScreenSource for SyntheticSource {
    fn size(&self) -> FrameSize {
        self.size
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        Some(self.grant.clone())
    }
    fn reference_generation(&self) -> u64 {
        self.grant.generation().value()
    }

    fn capture(&mut self, frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        self.image = None;
        let mut image = match CaptureImageBuilder::new(
            self.size,
            FrameLimits::LEGACY,
            &self.grant,
            CaptureOwner::SourcePixels,
        ) {
            Ok(image) => image,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(SourceFrame::Unchanged)
            }
            Err(error) => return Err(error),
        };
        render(frame_id, &mut image)?;
        let image = image.freeze();
        self.image = Some(image.clone());
        Ok(SourceFrame::Frame(image))
    }
}

impl Drop for SyntheticSource {
    fn drop(&mut self) {
        self.image = None;
        if !self.grant.is_member() {
            let _ = self.grant.retire();
        }
    }
}
