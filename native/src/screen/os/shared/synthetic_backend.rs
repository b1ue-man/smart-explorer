//! The screen host of SCR-M2 on every platform: the test picture
//! `synthetic` (C9) with the lossless tile codec, no screen capture and no
//! input yet. Capture, H.264 and input of Windows, Linux and Android follow
//! with SCR-M3 to SCR-M8 as backends of their own. A session that asks for a
//! monitor or window gets `no_display` with the reason `capture_unavailable`;
//! the host names that in its limits (`test_picture_only`).
use std::io;

use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceFrame, SourceRefusal, TilesScreenEncoder,
};
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::synthetic::{render, MIN_SYNTHETIC_SIZE};
use crate::screen::core::window_list::{ListSource, MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::HostPlatform;

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
    fn memory_facts(&self)->Option<crate::screen::core::frame_memory::MemoryFacts> {super::session_memory::facts()}
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
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        match source {
            SourceId::Synthetic => SyntheticSource::new(wanted)
                .map(|source| Box::new(source) as Box<dyn ScreenSource>)
                .map_err(|error| SourceRefusal::new(&error.to_string())),
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
    image: BgraImage,
}

impl SyntheticSource {
    /// At the viewer's size within 128 × 72 and 4096 × 2304, else 1280 × 720.
    pub(crate) fn new(wanted: Option<FrameSize>) -> io::Result<Self> {
        let size = wanted
            .filter(|size| {
                size.is_valid_frame()
                    && size.width >= MIN_SYNTHETIC_SIZE.width
                    && size.height >= MIN_SYNTHETIC_SIZE.height
            })
            .unwrap_or(DEFAULT_SYNTHETIC_SIZE);
        Ok(Self {
            image: BgraImage::new(size)?,
        })
    }
}

impl ScreenSource for SyntheticSource {
    fn size(&self) -> FrameSize {
        self.image.size()
    }

    fn capture(&mut self, frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        render(frame_id, &mut self.image)?;
        Ok(SourceFrame::Frame(self.image.clone()))
    }
}
