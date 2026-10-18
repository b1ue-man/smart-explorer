//! The host backend of the daemon with host processes (SCR-M3): a source
//! opens in a host process of its own (`WorkerSource`), so capture and
//! encoding run there; the encoders this host offers come from the codec
//! inventory of the user (Media Foundation, OpenH264, tiles); platform,
//! monitors, windows, limits and input stay with the platform backend.
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use super::{desktop_program, program_for, WorkerSource};
use crate::screen::core::codec_registry::CodecRegistry;
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceRefusal,
};
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecChoice, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::{MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::HostPlatform;

/// Frame rate of an encoder opened without one (`open_encoder`).
const DEFAULT_FPS: u32 = 30;

/// Where the host processes come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkerProgram {
    /// The desktop program next to this one; without it this program, for
    /// the test picture only.
    Installation {
        gui: Option<PathBuf>,
        current: Option<PathBuf>,
    },
    /// One program for every source with extra environment (task tests
    /// with a built `se`).
    #[cfg(test)]
    Fixed {
        path: PathBuf,
        env: Vec<(String, String)>,
    },
    /// No host process: capture and encoding in this process (task tests).
    #[cfg(test)]
    InProcess,
}

impl WorkerProgram {
    pub(crate) fn of_this_installation() -> Self {
        let (gui, current) = desktop_program();
        Self::Installation { gui, current }
    }
}

/// The share host's backend: host processes around the platform backend.
pub(crate) struct WorkerBackend {
    inner: Arc<dyn ScreenHostBackend>,
    program: WorkerProgram,
    /// OpenH264 store of the codecs; `None` is this user's.
    openh264_dir: Option<PathBuf>,
}

impl WorkerBackend {
    pub(crate) fn new(inner: Arc<dyn ScreenHostBackend>) -> Self {
        Self::with(inner, WorkerProgram::of_this_installation(), None)
    }

    pub(crate) fn with(
        inner: Arc<dyn ScreenHostBackend>,
        program: WorkerProgram,
        openh264_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            inner,
            program,
            openh264_dir,
        }
    }

    fn codecs(&self) -> CodecRegistry {
        super::super::codec_inventory::inventory(self.openh264_dir.as_deref(), false).registry
    }
}

impl ScreenHostBackend for WorkerBackend {
    fn platform(&self) -> HostPlatform {
        self.inner.platform()
    }

    fn graphical_session(&self) -> bool {
        self.inner.graphical_session()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        self.inner.monitors()
    }

    fn encoders(&self) -> Vec<CodecOffer> {
        self.codecs().encoder_offers()
    }

    fn input(&self) -> InputCapabilities {
        self.inner.input()
    }

    fn limits(&self) -> Vec<String> {
        self.inner.limits()
    }

    fn windows(&self) -> io::Result<WindowList> {
        self.inner.windows()
    }

    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let (program, env): (Option<&std::path::Path>, &[(String, String)]) = match &self.program {
            WorkerProgram::Installation { gui, current } => {
                (program_for(gui.as_deref(), current.as_deref(), source), &[])
            }
            #[cfg(test)]
            WorkerProgram::Fixed { path, env } => (Some(path.as_path()), env.as_slice()),
            #[cfg(test)]
            WorkerProgram::InProcess => return self.inner.open_source(source, wanted),
        };
        let Some(program) = program else {
            return Err(SourceRefusal::new(
                "host_process: the desktop program is not installed next to this program",
            ));
        };
        let opened = WorkerSource::open(program, env, source, wanted)?;
        Ok(Box::new(opened))
    }

    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(self
            .codecs()
            .open_encoder(choice, size, DEFAULT_FPS)?
            .encoder)
    }

    fn inject(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        self.inner.inject(events)
    }
}
