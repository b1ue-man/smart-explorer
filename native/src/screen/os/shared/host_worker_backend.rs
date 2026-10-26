//! The host backend of the daemon with host processes (SCR-M3): a source
//! opens in a host process of its own (`WorkerSource`), so capture and
//! encoding run there; the encoders this host offers come from the codec
//! inventory of the user (Media Foundation, OpenH264, tiles); platform,
//! monitors, windows, limits and the input capabilities stay with the
//! platform backend.
//!
//! Input (SCR-M6, decision of SEC on U16-4): the host process injects it,
//! the same process that captures (C5: `SendInput` at the user's integrity
//! level; on Wayland only the capture's portal session carries input). A
//! source opened for a session in the mode `control` is opened with
//! `control`; `inject` hands the batch to the newest such host process that
//! still runs. The session core calls `inject` only for the viewer that
//! holds control with the `ScreenControl` right. The mode of a session is
//! read from the share node's registry (`watch_registry`), because
//! `open_source` carries only the source.
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Instant;

use super::{desktop_program, program_for, SharedLink, WorkerSource, INPUT_TIMEOUT};
use crate::screen::core::codec_registry::CodecRegistry;
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceRefusal,
};
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecChoice, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::registry::ScreenRegistry;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::{MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::HostPlatform;
use crate::screen::core::wire_hello::SessionMode;
use crate::screen::core::worker_wire::{WorkerReply, WorkerRequest};

/// Frame rate of an encoder opened without one (`open_encoder`).
const DEFAULT_FPS: u32 = 30;

fn registry_slot() -> &'static Mutex<Weak<ScreenRegistry>> {
    static SLOT: OnceLock<Mutex<Weak<ScreenRegistry>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Weak::new()))
}

/// The share node's registry, whose sessions name their mode.
pub(crate) fn watch_registry(registry: &Arc<ScreenRegistry>) {
    *registry_slot()
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Arc::downgrade(registry);
}

/// Whether the session that is about to open `source` may control: the
/// newest registered session of this source that is starting or asked.
fn opens_for_control(source: &SourceId) -> bool {
    let registry = registry_slot()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .upgrade();
    let Some(registry) = registry else {
        return false;
    };
    let key = source.to_string();
    let (_, sessions) = registry.sessions();
    sessions
        .into_iter()
        .filter(|session| {
            session.source == key
                && matches!(
                    session.state,
                    SessionState::Starting | SessionState::WaitingForApproval
                )
        })
        .max_by_key(|session| session.started_at)
        .is_some_and(|session| session.mode == SessionMode::Control)
}

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
    /// The host processes of sessions that may control, newest last.
    controls: Mutex<Vec<Weak<Mutex<super::WorkerLink>>>>,
    families: Mutex<super::family_backend::FamilyPins>,
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
            controls: Mutex::new(Vec::new()),
            families: Mutex::new(super::family_backend::FamilyPins::default()),
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
    fn family_windows(&self, owner: &str) -> io::Result<crate::screen::core::family_protocol::FamilyListing> {
        // Fail explicitly on Wayland/Android rather than deriving app rights
        // from their normal portal/system-selected source lists.
        let list = self.inner.windows()?;
        self.families.lock().map_err(|_| io::Error::other("family_pin_store_poisoned"))?.list(owner, list)
    }
    fn register_family_pin(&self,owner:&str,request_id:&str,pin:crate::remote_window::WindowPin)
        ->io::Result<crate::screen::core::family_protocol::FamilyListEntry> {
        if let Some(entry)=self.families.lock().map_err(|_|io::Error::other("family_pin_store_poisoned"))?
            .receipt(owner,request_id,&pin)? {return Ok(entry);}
        let metadata=self.inner.windows()?.bounded();
        self.families.lock().map_err(|_|io::Error::other("family_pin_store_poisoned"))?.register(owner,request_id,pin,metadata)
    }
    fn open_session_source(&self, owner: &str, session: &str, source: &SourceId,
        wanted: Option<FrameSize>, control: bool) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        if crate::screen::core::family_protocol::selector(source).is_none() {
            return self.open_source(source, wanted);
        }
        let pinned = self.families.lock().map_err(|_| SourceRefusal::new("family_pin_store_poisoned"))?
            .take(owner, source).map_err(|error| SourceRefusal::new(&error.to_string()))?;
        let (program, env): (Option<&std::path::Path>, &[(String,String)]) = match &self.program {
            WorkerProgram::Installation {gui,current} => (program_for(gui.as_deref(),current.as_deref(),source), &[]),
            #[cfg(test)]
            WorkerProgram::Fixed {path,env} => (Some(path.as_path()), env.as_slice()),
            #[cfg(test)]
            WorkerProgram::InProcess => return Err(SourceRefusal::new("family_worker_required")),
        };
        let program = program.ok_or_else(|| SourceRefusal::new("family_host_program_unavailable"))?;
        Ok(Box::new(pinned.open(program, env, session, control, wanted)?))
    }
    fn icon(&self,entry:crate::screen::core::app_icon::AuthorizedIconEntry<'_>) -> io::Result<Option<crate::screen::core::app_icon::IconReply>> {
        self.inner.icon(entry)
    }

    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        if crate::screen::core::family_protocol::selector(source).is_some() {
            return Err(SourceRefusal::new("family_selection_requires_authenticated_session"));
        }
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
        let control = opens_for_control(source);
        let opened = WorkerSource::open(program, env, source, wanted, control)?;
        if control {
            let mut controls = self.controls.lock().unwrap_or_else(PoisonError::into_inner);
            controls.retain(|link| link.strong_count() > 0);
            controls.push(Arc::downgrade(&opened.shared_link()));
        }
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

    fn open_encoder_for_stage(&self, request: &crate::screen::core::host_stage::StageRequest) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(self.codecs().open_encoder_with_contract(&request.choice, request.size, request.fps, &[], request.frame_contract)?.encoder)
    }

    fn inject(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        #[cfg(test)]
        if self.program == WorkerProgram::InProcess {
            return self.inner.inject(events);
        }
        let link: Option<SharedLink> = self
            .controls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .rev()
            .find_map(Weak::upgrade);
        let link = link.ok_or(ControlBlock::ControlUnavailable)?;
        send_bound_input(&link, events,None,false)
    }
}

/// One input batch to a host process: waits at most `INPUT_TIMEOUT` for a
/// running picture request, then at most as long for the answer (a host
/// process that does not answer is ended, like for a picture). Nothing of
/// the batch is logged.
pub(super) fn send_bound_input(link: &SharedLink, events: &[InputEvent],reference:Option<u64>,presented:bool) -> Result<(), ControlBlock> {
    let deadline = Instant::now() + INPUT_TIMEOUT;
    let mut guard = loop {
        match link.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => break poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(ControlBlock::ControlUnavailable)
            }
        }
    };
    let input=crate::screen::core::worker_wire::BoundInput {events:events.to_vec(),
        reference_generation:reference.unwrap_or_else(||guard.facts.generation.load(std::sync::atomic::Ordering::Acquire)),presented};
    match guard.ask(&WorkerRequest::Input(input), INPUT_TIMEOUT) {
        Ok(WorkerReply::InputDone { blocked: None,.. }) => Ok(()),
        Ok(WorkerReply::InputDone {
            blocked: Some(blocked),
            ..
        }) => Err(blocked),
        _ => Err(ControlBlock::ControlUnavailable),
    }
}
