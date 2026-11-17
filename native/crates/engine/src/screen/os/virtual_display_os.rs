//! Platform side of the virtual display in the viewer's resolution (plan
//! 2026-11-17 P5, `core/virtual_display.rs`): Linux X11 (RandR modes, the
//! Xorg driver `dummy` with one elevation) and the Wayland portal's virtual
//! source; a Windows host names its pending display driver
//! (`virtual_display_unsupported:windows_driver_pending`, decision E6: an
//! indirect display driver needs a decision on the code-signing
//! certificate; the hello fields carry it later without a format change);
//! other platforms have none. The share host's backend hands the driver to
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

/// A Windows host answers that its driver is pending.
#[cfg(windows)]
pub fn driver() -> Option<Arc<dyn VirtualDisplayDriver>> {
    Some(Arc::new(windows_pending::WindowsPending))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn driver() -> Option<Arc<dyn VirtualDisplayDriver>> {
    None
}

#[cfg(windows)]
mod windows_pending {
    use std::io;

    use crate::screen::core::pixel_convert::FrameSize;
    use crate::screen::core::virtual_display::{DriverLimit, WINDOWS_DRIVER_PENDING};
    use crate::screen::core::virtual_display_host::{
        DisplayProbe, SetupProgress, VirtualDisplayDriver,
    };

    pub struct WindowsPending;

    impl VirtualDisplayDriver for WindowsPending {
        fn probe(&self) -> io::Result<DisplayProbe> {
            Ok(DisplayProbe {
                physical_monitor: false,
                modes: false,
                limit: DriverLimit::NONE,
                driver: "windows".into(),
                current: None,
                unsupported: Some(WINDOWS_DRIVER_PENDING.into()),
                setup_possible: false,
                portal: false,
            })
        }

        fn apply(&self, _size: FrameSize) -> io::Result<FrameSize> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                WINDOWS_DRIVER_PENDING,
            ))
        }

        fn request_setup(&self) -> SetupProgress {
            SetupProgress::Failed(WINDOWS_DRIVER_PENDING.into())
        }
    }
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
