//! The Linux side of the virtual display (plan 2026-11-17 P5). X11: the
//! modes of RandR 1.2 (`x11_randr_modes`, stage 1) within the limit of the
//! driver (screen size range; bochs half of its video memory; `dummy` its
//! `VideoRam`), and the setup of the driver `dummy` (`setup`, stage 2) for a
//! server without modes. The X server is changed by a process with the
//! session's `DISPLAY` and `XAUTHORITY` (decision E3): the worker itself when
//! it has its own session variables, otherwise a short host process in the
//! followed session's account (session query `display`, E47), never a root
//! process against that account's X server. Wayland: the compositor makes
//! the display through the portal's virtual source in the host process
//! (`portal_source_binding.rs`); wlroots desktops without a headless output
//! are the limit.
use std::io;

use x11rb::connection::Connection;

use super::{probe, setup, x11_randr_modes};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::virtual_display::{DriverLimit, PhysicalFacts};
use crate::screen::core::virtual_display_config::VIDEO_RAM_KIB;
use crate::screen::core::virtual_display_host::{
    DisplayFacts, DisplayProbe, SetupProgress, VirtualDisplayDriver,
};
use crate::screen::os::host_worker_client::session_queries::{self, SessionAnswer, SessionQuery};
use crate::screen::os::session_detect::{self, SessionKind};

/// The virtual display of this Linux host.
pub struct LinuxVirtualDisplay;

/// Whether this process has session variables of its own (then it reaches
/// its X server itself).
fn own_session() -> bool {
    session_detect::active_session::spawn::has_own_session(&|name| std::env::var(name).ok())
}

/// `XDG_CURRENT_DESKTOP` of this process, or of the followed session.
fn desktop() -> String {
    if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
        return desktop;
    }
    session_detect::active_session::current()
        .ok()
        .and_then(|session| {
            session
                .env
                .iter()
                .find(|(name, _)| name == "XDG_CURRENT_DESKTOP")
                .map(|(_, value)| value.clone())
        })
        .unwrap_or_default()
}

/// This process's X server (its own `DISPLAY` and `XAUTHORITY`): the display
/// set to `size`, or its monitor's own mode given back with `restore`.
fn local_answer(
    size: Option<FrameSize>,
    restore: bool,
) -> io::Result<(DisplayFacts, Option<FrameSize>)> {
    let (conn, screen) = session_detect::active_session::x11::connect()?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .map(|screen| screen.root)
        .ok_or_else(|| io::Error::other("x11: the screen of DISPLAY does not exist"))?;
    let applied = match (size, restore) {
        (Some(size), _) => Some(x11_randr_modes::apply(&conn, root, size)?),
        (None, true) => x11_randr_modes::restore(&conn, root)?,
        (None, false) => None,
    };
    Ok((x11_randr_modes::probe(&conn, root)?, applied))
}

/// The session query `display` in the session's account.
pub fn answer_query(
    size: Option<FrameSize>,
    restore: bool,
) -> io::Result<(DisplayFacts, Option<FrameSize>)> {
    if !own_session() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "the session query has no session variables",
        ));
    }
    local_answer(size, restore)
}

/// The X server of the session: here, or through a host process in the
/// followed session's account.
fn x11(size: Option<FrameSize>, restore: bool) -> io::Result<(DisplayFacts, Option<FrameSize>)> {
    if own_session() {
        return local_answer(size, restore);
    }
    match session_queries::ask(&SessionQuery::Display { size, restore })? {
        SessionAnswer::Display { facts, size } => Ok((facts, size)),
        SessionAnswer::Failed(message) => Err(io::Error::other(message)),
        _ => Err(io::Error::other("unexpected screen session answer")),
    }
}

fn probe_x11(mut physical: PhysicalFacts) -> io::Result<DisplayProbe> {
    let (facts, _) = x11(None, false)?;
    physical.display_outputs = facts.outputs.clone();
    let physical_monitor = physical.physical_monitor();
    let dummy = facts
        .outputs
        .iter()
        .any(|name| name.to_ascii_lowercase().starts_with("dummy"));
    let bochs = if dummy {
        None
    } else {
        probe::bochs_video_memory()
    };
    let mut limit = DriverLimit::sides(facts.max_width, facts.max_height);
    if let Some(memory) = bochs {
        limit = limit.narrowed(DriverLimit::bochs(memory));
    }
    if dummy {
        limit = limit.narrowed(DriverLimit::dummy(VIDEO_RAM_KIB));
    }
    let driver = if dummy {
        "dummy"
    } else if !facts.modes && probe::drm_driver().is_none() {
        "vesa"
    } else if bochs.is_some() {
        "bochs"
    } else if facts.modes {
        "randr"
    } else {
        "x11"
    };
    Ok(DisplayProbe {
        physical_monitor,
        modes: facts.modes,
        limit,
        driver: driver.into(),
        current: Some(facts.current),
        unsupported: (!facts.modes).then(|| "x11_without_display_modes".to_string()),
        setup_possible: !dummy && !physical_monitor && setup::possible(),
        portal: false,
    })
}

fn without_modes(physical_monitor: bool, driver: &str, reason: Option<&str>) -> DisplayProbe {
    DisplayProbe {
        physical_monitor,
        modes: false,
        limit: DriverLimit::NONE,
        driver: driver.into(),
        current: None,
        unsupported: reason.map(str::to_string),
        setup_possible: false,
        portal: false,
    }
}

impl VirtualDisplayDriver for LinuxVirtualDisplay {
    fn probe(&self) -> io::Result<DisplayProbe> {
        let physical = probe::physical_facts();
        match session_detect::detect_process() {
            SessionKind::X11 => probe_x11(physical),
            SessionKind::Wayland => {
                let physical_monitor = physical.physical_monitor();
                if session_detect::is_wlroots_desktop(&desktop()) {
                    Ok(without_modes(
                        physical_monitor,
                        "portal",
                        Some("wlroots_without_headless"),
                    ))
                } else {
                    let mut probe = without_modes(physical_monitor, "portal", None);
                    probe.portal = true;
                    Ok(probe)
                }
            }
            SessionKind::None => Ok(without_modes(
                physical.physical_monitor(),
                "none",
                Some("no_graphical_session"),
            )),
        }
    }

    fn apply(&self, size: FrameSize) -> io::Result<FrameSize> {
        let (_, applied) = x11(Some(size), false)?;
        applied.ok_or_else(|| io::Error::other("the display answered without a size"))
    }

    fn request_setup(&self) -> SetupProgress {
        setup::request()
    }

    /// Only an X server has se's modes; a Wayland compositor ends its own
    /// virtual monitor with the portal session.
    fn restore(&self) -> io::Result<()> {
        if session_detect::detect_process() == SessionKind::X11 {
            x11(None, true)?;
        }
        Ok(())
    }
}
