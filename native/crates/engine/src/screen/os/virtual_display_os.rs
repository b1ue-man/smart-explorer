//! Platform side of the virtual display in the viewer's resolution (plan
//! 2026-11-17 P5, `core/virtual_display.rs`): Linux X11 (RandR modes, the
//! Xorg driver `dummy` with one elevation) and the Wayland portal's virtual
//! source; Windows the indirect display driver of VirtualDrivers, signed on
//! the device and installed by the elevated step of the installation or an
//! update (`windows/virtual_display_*.rs`, user decision 2026-11-17: never a
//! setup step during use), whose monitor joins the desktop as the primary
//! one; other platforms have none. The share host's backend hands the driver to
//! the sessions (`WorkerBackend::virtual_display`). The host process of a
//! Wayland session learns the size its portal offers with its preparation
//! (`set_portal_request`); a worker that follows a session asks the
//! session's account for its X server (`answer_query`, E47).
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::virtual_display_host::{DisplayFacts, VirtualDisplayDriver};

#[cfg(target_os = "linux")]
#[path = "linux_os/virtual_display_driver.rs"]
mod linux;
#[cfg(target_os = "linux")]
#[path = "linux_os/virtual_display_probe.rs"]
pub mod probe;
#[cfg(target_os = "linux")]
#[path = "linux_os/virtual_display_setup.rs"]
pub mod setup;
#[cfg(target_os = "linux")]
#[path = "linux_os/x11_randr_modes.rs"]
pub mod x11_randr_modes;

/// The virtual display of this host.
#[cfg(target_os = "linux")]
pub fn driver() -> Option<Arc<dyn VirtualDisplayDriver>> {
    Some(Arc::new(linux::LinuxVirtualDisplay))
}

#[cfg(windows)]
#[path = "windows/virtual_display_device.rs"]
pub mod device;
#[cfg(windows)]
#[path = "windows/virtual_display_elevate.rs"]
pub mod elevate;
#[cfg(windows)]
#[path = "windows/virtual_display_install.rs"]
pub mod install;
#[cfg(windows)]
#[path = "windows/virtual_display_pins.rs"]
pub mod pins;
#[cfg(windows)]
#[path = "windows/virtual_display_sign.rs"]
pub mod sign;
#[cfg(windows)]
#[path = "windows/virtual_display_driver.rs"]
pub mod win_display;

/// The virtual display of a Windows host (driver of VirtualDrivers).
#[cfg(windows)]
pub fn driver() -> Option<Arc<dyn VirtualDisplayDriver>> {
    Some(Arc::new(win_display::WindowsVirtualDisplay))
}

/// The driver step of the updater program (`--display-driver …`); `None`
/// for other arguments and on other platforms.
#[cfg(windows)]
pub fn driver_step_command(args: &[String]) -> Option<i32> {
    elevate::command(args)
}

#[cfg(not(windows))]
pub fn driver_step_command(_args: &[String]) -> Option<i32> {
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn driver() -> Option<Arc<dyn VirtualDisplayDriver>> {
    None
}

/// The size the portal of this host process offers for its virtual source
/// (`CaptureRequest::Prepare.virtual_display`); one process serves one
/// session.
static PORTAL_REQUEST: Mutex<Option<FrameSize>> = Mutex::new(None);

/// Host process: the preparation names the size of the virtual source, or
/// none (a source as before).
pub fn set_portal_request(size: Option<FrameSize>) {
    *PORTAL_REQUEST
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = size;
}

/// Host process: the size to offer the Wayland portal's virtual source in,
/// if the session asked for se's own display.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn portal_request() -> Option<FrameSize> {
    *PORTAL_REQUEST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The session query of a worker that follows a session (E47), answered
/// in the session's account: the facts of its X server, after the display
/// was set to `size` or its monitor got its own mode back (`restore`), and
/// the size it shows then.
#[cfg(target_os = "linux")]
pub fn answer_query(
    size: Option<FrameSize>,
    restore: bool,
) -> io::Result<(DisplayFacts, Option<FrameSize>)> {
    linux::answer_query(size, restore)
}

#[cfg(not(target_os = "linux"))]
pub fn answer_query(
    _size: Option<FrameSize>,
    _restore: bool,
) -> io::Result<(DisplayFacts, Option<FrameSize>)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "virtual_display_unsupported",
    ))
}

/// `se rdp host-setup --system virtual-display-…` (the elevated side).
#[cfg(target_os = "linux")]
pub fn run_system_step(step: &str) -> (String, i32) {
    setup::run_system_step(step)
}

#[cfg(not(target_os = "linux"))]
pub fn run_system_step(_step: &str) -> (String, i32) {
    ("display_state=failed:unsupported_platform".into(), 55)
}

/// A „Bildschirm steuern“ right ended; after the last one se's driver files
/// go (`rdp::host_setup::coverage_ended`, as the login self-setup).
#[cfg(target_os = "linux")]
pub fn coverage_ended(steering_remains: bool) {
    setup::coverage_ended(steering_remains);
}

#[cfg(not(target_os = "linux"))]
pub fn coverage_ended(_steering_remains: bool) {}
