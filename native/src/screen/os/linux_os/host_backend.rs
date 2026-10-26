//! The screen host of Linux (SCR-M5): X11 sessions are captured with MIT-SHM
//! (`x11_capture`), Wayland sessions through the portal and PipeWire
//! (`portal_session`); the window lists come from the session's own
//! interfaces (`x11_windows`, `wayland_toplevels`, `kde_window_management`,
//! `app_units`). The backend works in the daemon (monitors, windows, limits)
//! and in the host process (sources and input, SCR-M6): X11 takes input
//! through XTest (`x11_input`), Wayland through the RemoteDesktop portal in
//! the capture's own portal session (`portal_input`), which a session that
//! may control asks for; wlroots desktops have no RemoteDesktop portal, so
//! there input is `control_unavailable`.
//! The test picture `synthetic` is delegated unchanged. A process without a
//! display, without PipeWire or without a window-list interface starts and
//! says so in its limits.
use std::io;
use std::path::PathBuf;
use std::sync::{Arc,OnceLock};
#[path="capture_prepared.rs"]
pub(super) mod prepared;

use super::portal_session::{PortalOptions, PortalSource};
use super::portal_tokens::{self, TokenKind};
use super::session_detect::{self, SessionKind};
use super::{app_units, kde_window_management, pipewire_abi, wayland_toplevels};
use super::{x11_capture, x11_monitors, x11_windows};
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceRefusal,
};
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecChoice, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::{ListSource, MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::{HostPlatform, LIMIT_CONSENT_EACH_SESSION};
use crate::screen::os::synthetic_backend::SyntheticBackend;

/// Id, name and size of the one Wayland „monitor“: the portal dialog picks
/// the screen, so width and height 0 mean „unknown until chosen“.
pub(crate) const PORTAL_MONITOR_ID: &str = "portal";
const PORTAL_MONITOR_NAME: &str = "Auswahl am Gerät";
pub(crate) const LIMIT_WINDOW_PICK_ON_HOST: &str = "wayland_window_pick_on_host";
pub(crate) const LIMIT_PERSIST_NEEDS_XDPW_ENV: &str = "portal_persist_needs_xdpw_env";
pub(crate) const LIMIT_PIPEWIRE_MISSING: &str = "pipewire_missing";
pub(crate) const LIMIT_X11_NO_SHM: &str = "x11_no_shm";
pub(crate) const LIMIT_X11_OVERLAPS: &str = "x11_window_overlaps_without_compositor";
pub(crate) const LIMIT_NO_WINDOW_LIST: &str = "no_window_list";
pub(crate) const LIMIT_KDE_NEEDS_DESKTOP_APP: &str = "kde_window_list_needs_desktop_app";
/// Reason of a refusal to open a window or another monitor on Wayland.
pub(crate) const PORTAL_SELECTS_SOURCE: &str = "portal_selects_source";

/// Whether `program` is the terminal program `se` (not the desktop program).
pub(crate) fn is_terminal_program(program: &std::path::Path) -> bool {
    program.file_name().is_some_and(|name| name == "se")
}

/// The Linux screen host.
pub(crate) struct LinuxBackend {
    session: SessionKind,
    pipewire_library: &'static str,
    /// Token file of the tests; `None` is this user's.
    tokens_file: Option<PathBuf>,
    x11_facts: OnceLock<Option<x11_capture::ServerFacts>>,
    no_window_list: OnceLock<bool>,
}

impl LinuxBackend {
    /// The backend of this process's session.
    pub(crate) fn new() -> Self {
        Self::with(
            session_detect::detect_process(),
            pipewire_abi::LIBRARY_NAME,
            None,
        )
    }

    pub(crate) fn with(
        session: SessionKind,
        pipewire_library: &'static str,
        tokens_file: Option<PathBuf>,
    ) -> Self {
        Self {
            session,
            pipewire_library,
            tokens_file,
            x11_facts: OnceLock::new(),
            no_window_list: OnceLock::new(),
        }
    }

    fn has_stored_token(&self) -> bool {
        let tokens = match &self.tokens_file {
            Some(file) => portal_tokens::load_from(file),
            None => portal_tokens::load(),
        };
        tokens.get(TokenKind::ScreenCast).is_some()
    }

    fn x11_facts(&self) -> Option<&x11_capture::ServerFacts> {
        self.x11_facts.get_or_init(x11_capture::probe).as_ref()
    }

    /// The list from the first interface the session offers.
    fn list_windows(&self) -> io::Result<WindowList> {
        let mut list = match self.session {
            SessionKind::None => unknown_list(),
            SessionKind::X11 => x11_windows::list()?,
            SessionKind::Wayland => wayland_list(),
        };
        if self.session == SessionKind::Wayland {
            list.limits.push(LIMIT_WINDOW_PICK_ON_HOST.into());
            if kde_desktop_without_desktop_app() {
                list.limits.push(LIMIT_KDE_NEEDS_DESKTOP_APP.into());
            }
        }
        if list.list_source == ListSource::Unknown {
            list.limits.push(LIMIT_NO_WINDOW_LIST.into());
        }
        Ok(list)
    }
}

fn unknown_list() -> WindowList {
    WindowList {
        windows: Vec::new(),
        apps: Vec::new(),
        list_source: ListSource::Unknown,
        limits: Vec::new(),
    }
}

fn desktop_is(name: &str) -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktop| desktop.to_ascii_lowercase().contains(name))
}

/// KDE does not give the window interface to `se`.
fn kde_desktop_without_desktop_app() -> bool {
    desktop_is("kde") && std::env::current_exe().is_ok_and(|program| is_terminal_program(&program))
}

/// Compositor protocols first (ext, wlr, KDE), then systemd units.
fn wayland_list() -> WindowList {
    if let Ok(Some(list)) = wayland_toplevels::list() {
        return list;
    }
    if !kde_desktop_without_desktop_app() {
        if let Ok(Some(list)) = kde_window_management::list() {
            return list;
        }
    }
    app_units::list().unwrap_or_else(|_| unknown_list())
}

impl ScreenHostBackend for LinuxBackend {
    fn memory_facts(&self)->Option<crate::screen::core::frame_memory::MemoryFacts> {super::session_memory::facts()}
    fn platform(&self) -> HostPlatform {
        HostPlatform::Linux
    }

    fn graphical_session(&self) -> bool {
        self.session != SessionKind::None
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        match self.session {
            SessionKind::None => Vec::new(),
            SessionKind::Wayland => vec![MonitorInfo {
                id: PORTAL_MONITOR_ID.into(),
                name: PORTAL_MONITOR_NAME.into(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                primary: true,
            }],
            SessionKind::X11 => x11_rb_monitors().unwrap_or_default(),
        }
    }

    fn encoders(&self) -> Vec<CodecOffer> {
        SyntheticBackend.encoders()
    }

    fn input(&self) -> InputCapabilities {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        input_for(self.session, &desktop)
    }

    fn limits(&self) -> Vec<String> {
        let mut limits = Vec::new();
        match self.session {
            SessionKind::Wayland => {
                if !self.has_stored_token() {
                    limits.push(LIMIT_CONSENT_EACH_SESSION.into());
                }
                limits.push(LIMIT_WINDOW_PICK_ON_HOST.into());
                let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
                if session_detect::is_wlroots_desktop(&desktop) {
                    limits.push(LIMIT_PERSIST_NEEDS_XDPW_ENV.into());
                }
                if !pipewire_abi::available(self.pipewire_library) {
                    limits.push(LIMIT_PIPEWIRE_MISSING.into());
                }
                if kde_desktop_without_desktop_app() {
                    limits.push(LIMIT_KDE_NEEDS_DESKTOP_APP.into());
                }
            }
            SessionKind::X11 => {
                if let Some(facts) = self.x11_facts() {
                    if !facts.shm {
                        limits.push(LIMIT_X11_NO_SHM.into());
                    }
                    if !facts.compositor {
                        limits.push(LIMIT_X11_OVERLAPS.into());
                    }
                }
            }
            SessionKind::None => {}
        }
        let none = *self.no_window_list.get_or_init(|| {
            self.list_windows()
                .map_or(true, |list| list.list_source == ListSource::Unknown)
        });
        if none {
            limits.push(LIMIT_NO_WINDOW_LIST.into());
        }
        limits
    }

    fn windows(&self) -> io::Result<WindowList> {
        let mut list=self.list_windows()?;
        super::app_icons::attach(&mut list);
        Ok(list)
    }

    fn icon(&self, entry: crate::screen::core::app_icon::AuthorizedIconEntry<'_>) -> io::Result<Option<crate::screen::core::app_icon::IconReply>> {
        super::app_icons::load(entry)
    }

    fn prepare_session_source(&self,owner:&str,session:&str,source:&SourceId,wanted:Option<FrameSize>,
        control:bool,admission:Arc<crate::screen::core::capture_admission::SessionAdmission>)
        ->Result<Box<dyn crate::screen::core::host_backend::PreparedSource>,SourceRefusal> {
        if admission.session()!=session {return Err(SourceRefusal::new("capture_session_mismatch"));}
        if matches!(source,SourceId::Synthetic) {
            return SyntheticBackend.prepare_session_source(owner,session,source,wanted,control,admission);
        }
        let key=crate::screen::core::capture_admission::SourceKey::new(session,&format!("linux:{source:?}"))
            .map_err(|error|SourceRefusal::new(&error.to_string()))?;
        match self.session {
            SessionKind::None=>Err(SourceRefusal::new("no_display")),
            SessionKind::X11=>prepared::X11Prepared::new(source,key,admission,false)
                .map(|source|Box::new(source) as Box<dyn crate::screen::core::host_backend::PreparedSource>),
            SessionKind::Wayland=>match source {
                SourceId::Monitor(id) if id==PORTAL_MONITOR_ID=>{
                    let desktop=std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
                    crate::screen::core::host_admission::prepare_registered(
                        key.clone(),admission.clone(),|| {
                            prepared::PortalPrepared::new(PortalOptions {
                                remote_desktop:control && remote_desktop_portal(&desktop),
                                pipewire_library:self.pipewire_library},key,admission)
                                .map(|source|Box::new(source) as Box<dyn crate::screen::core::host_backend::PreparedSource>)
                                .map_err(|error|SourceRefusal::new(&format!("portal: {error}")))
                        })
                }
                _=>Err(SourceRefusal::new(PORTAL_SELECTS_SOURCE)),
            }
        }
    }
    fn prepare_family_member(&self,_owner:&str,session:&str,native:crate::remote_window::NativeWindowId,
        admission:Arc<crate::screen::core::capture_admission::SessionAdmission>)
        ->Result<Box<dyn crate::screen::core::host_backend::PreparedSource>,SourceRefusal> {
        if admission.session()!=session {return Err(SourceRefusal::new("capture_session_mismatch"));}
        match self.session {
            SessionKind::X11=>{
                let source=SourceId::Window(native.0.to_string());
                let key=crate::screen::core::capture_admission::SourceKey::new(session,&format!("linux:{source:?}"))
                    .map_err(|error|SourceRefusal::new(&error.to_string()))?;
                prepared::X11Prepared::new(&source,key,admission,true)
                    .map(|source|Box::new(source) as Box<dyn crate::screen::core::host_backend::PreparedSource>)
            }
            SessionKind::Wayland=>Err(SourceRefusal::new(PORTAL_SELECTS_SOURCE)),
            SessionKind::None=>Err(SourceRefusal::new("no_display")),
        }
    }

    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let _=wanted;
        if self.session==SessionKind::None && !matches!(source,SourceId::Synthetic) {
            return Err(SourceRefusal::new("no_display"));
        }
        if self.session==SessionKind::Wayland && !matches!(source,SourceId::Synthetic)
            && !matches!(source,SourceId::Monitor(id) if id==PORTAL_MONITOR_ID) {
            return Err(SourceRefusal::new(PORTAL_SELECTS_SOURCE));
        }
        Err(SourceRefusal::new("capture_grant_required"))
    }

    fn open_family_member(&self,_native:crate::remote_window::NativeWindowId)
        ->Result<Box<dyn ScreenSource>,SourceRefusal> {
        Err(SourceRefusal::new("capture_grant_required"))
    }

    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        SyntheticBackend.open_encoder(choice, size)
    }

    fn open_encoder_for_stage(&self, request: &crate::screen::core::host_stage::StageRequest) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(crate::screen::os::codec_inventory::registry().open_encoder_with_contract(&request.choice, request.size, request.fps, &[], request.frame_contract)?.encoder)
    }

    fn open_encoder_for_stage_admitted(&self,request:&crate::screen::core::host_stage::StageRequest,
        grant:crate::screen::core::capture_admission::CaptureGrant)->io::Result<Box<dyn ScreenEncoder>> {
        Ok(crate::screen::os::codec_inventory::registry()
            .open_encoder_for_stage_admitted(request,&[],grant)?.encoder)
    }

    fn inject(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        match self.session {
            SessionKind::X11 => super::x11_input::inject(events),
            SessionKind::Wayland => super::portal_input::inject(events),
            SessionKind::None => Err(ControlBlock::UnsupportedOnHost),
        }
    }
    fn inject_family(&self, member: &crate::remote_window::CaptureMember,
        events: &[InputEvent]) -> Result<(), ControlBlock> {
        if self.session == SessionKind::X11 {
            inject_x11_family(member, events, super::x11_input::inject_bound)
        } else {
            self.inject(events)
        }
    }
    fn release_input(&self,events:&[InputEvent])->Result<(),ControlBlock> {
        match self.session {
            SessionKind::X11=>super::x11_input::release(events),
            // Keep the actual portal session/device grant boundary unchanged.
            SessionKind::Wayland=>super::portal_input::inject(events),
            SessionKind::None=>Err(ControlBlock::UnsupportedOnHost),
        }
    }
}

/// Whether `desktop` (`XDG_CURRENT_DESKTOP`) has a RemoteDesktop portal:
/// GNOME and KDE do, wlroots desktops (Sway, Hyprland, river, Wayfire,
/// phosh) do not (NR N11).
pub(crate) fn remote_desktop_portal(desktop: &str) -> bool {
    !session_detect::is_wlroots_desktop(desktop)
}

/// The input this host offers in `session` on `desktop` (C5).
pub(crate) fn input_for(session: SessionKind, desktop: &str) -> InputCapabilities {
    let full = |touch: bool| InputCapabilities {
        pointer: true,
        keyboard: true,
        text: true,
        touch,
        actions: Vec::new(),
        toolbar_keys: crate::screen::os::input_keys::toolbar_keys(),
    };
    match session {
        SessionKind::X11 => full(true),
        SessionKind::Wayland if remote_desktop_portal(desktop) => full(true),
        SessionKind::Wayland | SessionKind::None => InputCapabilities::default(),
    }
}

/// The monitors of the X server of `DISPLAY`.
fn x11_rb_monitors() -> io::Result<Vec<MonitorInfo>> {
    use x11rb::connection::Connection;
    let (connection, screen) =
        x11rb::rust_connection::RustConnection::connect(None).map_err(io::Error::other)?;
    let root = connection.setup().roots[screen].root;
    x11_monitors::list(&connection, root)
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;

/// Keep the host gate's admitted member geometry through the final XTest
/// boundary. A movement/resize after that gate is denied by inject_bound.
fn inject_x11_family(member: &crate::remote_window::CaptureMember, events: &[InputEvent],
    inject: impl FnOnce(&SourceId, super::x11_input::Area, &[InputEvent]) -> Result<(), ControlBlock>)
    -> Result<(), ControlBlock>
{
    if !member.input || !member.geometry.valid() { return Err(ControlBlock::ControlUnavailable); }
    let window = u32::try_from(member.binding.native.0).ok().filter(|id| *id > 1)
        .ok_or(ControlBlock::ControlUnavailable)?;
    let area = super::x11_input::Area { x: member.geometry.x, y: member.geometry.y,
        width: i32::try_from(member.geometry.width).map_err(|_| ControlBlock::ControlUnavailable)?,
        height: i32::try_from(member.geometry.height).map_err(|_| ControlBlock::ControlUnavailable)? };
    inject(&SourceId::Window(window.to_string()), area, events)
}

#[cfg(test)]
mod batches_family_geometry_task_tests {
    use super::*;
    use crate::remote_window::{CaptureGeometry, CaptureMember, NativeWindowId, WindowBinding, WindowIdentity};

    fn member() -> CaptureMember {
        CaptureMember { identity: WindowIdentity { session: "ab".repeat(16), generation: 2 },
            binding: WindowBinding { native: NativeWindowId(42), birth: vec![7; 16] },
            geometry: CaptureGeometry { x: -50, y: 20, width: 321, height: 123 }, z_order: 1, input: true }
    }

    #[test]
    fn batches_family_x11_input_retains_admitted_member_geometry_until_inject_bound() {
        let member = member();
        let events = vec![InputEvent::PointerAbs { x: u16::MAX, y: 0 },
            InputEvent::Touch { id: 0, phase: crate::screen::core::input_model::TouchPhase::Start, x: 0, y: u16::MAX }];
        let mut called = false;
        let result = inject_x11_family(&member, &events, |source, area, forwarded| {
            called = true;
            assert_eq!(source, &SourceId::Window("42".into()));
            assert_eq!(area, super::super::x11_input::Area { x: -50, y: 20, width: 321, height: 123 });
            assert_eq!(forwarded, events.as_slice());
            // A final native geometry refusal must reach WorkerFamily so it
            // releases held input; it must not fall back to unbound injection.
            Err(ControlBlock::ControlUnavailable)
        });
        assert!(called); assert_eq!(result, Err(ControlBlock::ControlUnavailable));
    }

    #[test]
    fn batches_family_x11_invalid_member_geometry_never_reaches_the_injector() {
        let mut cases = vec![member(); 4];
        cases[0].input = false;
        cases[1].geometry.width = 0;
        cases[2].geometry.x = -1; cases[2].geometry.width = i32::MAX as u32 + 1;
        cases[3].binding.native = NativeWindowId(u64::from(u32::MAX) + 1);
        for member in cases {
            assert_eq!(inject_x11_family(&member, &[], |_, _, _| panic!("invalid member reached XTest")),
                Err(ControlBlock::ControlUnavailable));
        }
    }
}
