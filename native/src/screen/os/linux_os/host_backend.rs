//! The screen host of Linux (SCR-M5): X11 sessions are captured with MIT-SHM
//! (`x11_capture`), Wayland sessions through the portal and PipeWire
//! (`portal_session`); the window lists come from the session's own
//! interfaces (`x11_windows`, `wayland_toplevels`, `kde_window_management`,
//! `app_units`). The backend works in the daemon (monitors, windows, limits)
//! and in the host process (sources). Input (XTest, the RemoteDesktop portal)
//! follows with U22: until then `input()` offers nothing and `inject` refuses.
//! The test picture `synthetic` is delegated unchanged. A process without a
//! display, without PipeWire or without a window-list interface starts and
//! says so in its limits.
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

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
        InputCapabilities::default()
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
        self.list_windows()
    }

    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        if matches!(source, SourceId::Synthetic) {
            return SyntheticBackend.open_source(source, wanted);
        }
        match self.session {
            SessionKind::None => Err(SourceRefusal::new("no_display")),
            SessionKind::X11 => x11_capture::X11Source::open(source)
                .map(|opened| Box::new(opened) as Box<dyn ScreenSource>),
            SessionKind::Wayland => match source {
                SourceId::Monitor(id) if id == PORTAL_MONITOR_ID => {
                    let options = PortalOptions {
                        remote_desktop: false,
                        pipewire_library: self.pipewire_library,
                    };
                    PortalSource::open(options)
                        .map(|opened| Box::new(opened) as Box<dyn ScreenSource>)
                        .map_err(|error| SourceRefusal::new(&format!("portal: {error}")))
                }
                _ => Err(SourceRefusal::new(PORTAL_SELECTS_SOURCE)),
            },
        }
    }

    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        SyntheticBackend.open_encoder(choice, size)
    }

    fn inject(&self, _events: &[InputEvent]) -> Result<(), ControlBlock> {
        Err(ControlBlock::UnsupportedOnHost)
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
