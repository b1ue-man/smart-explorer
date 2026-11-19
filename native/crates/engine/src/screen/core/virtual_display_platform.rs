//! The platform side of the virtual display (plan 2026-11-17 P5): what a
//! platform finds out about its display, the driver that changes it, and
//! the answer of a probe session (`capability`). `virtual_display_host.rs`
//! runs the sessions with it; the platforms implement it
//! (`screen/os/virtual_display_os.rs`).
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::super::pixel_convert::FrameSize;
use super::super::virtual_display::{
    DriverLimit, VirtualDisplayReport, VirtualDisplayState, REASON_AVAILABLE, REASON_PHYSICAL,
    REASON_SETUP,
};

/// What a host process found on its X server (`x11_randr_modes::probe`),
/// also the answer of the session query of a worker that follows a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayFacts {
    /// RandR 1.2 with an output that takes modes.
    pub modes: bool,
    /// The X server's screen size range.
    pub max_width: u32,
    pub max_height: u32,
    /// The size of the root window now.
    pub current: FrameSize,
    /// Names of the connected outputs (`DUMMY0`, `Virtual-1`, `DP-0`, …).
    pub outputs: Vec<String>,
}

/// What a platform found out about its display now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayProbe {
    /// `PhysicalFacts::physical_monitor`.
    pub physical_monitor: bool,
    /// The display server takes se's modes (X11 RandR 1.2 outputs).
    pub modes: bool,
    /// The largest display the driver takes.
    pub limit: DriverLimit,
    /// `randr`, `dummy`, `bochs`, `vesa`, `portal`, `windows`, …
    pub driver: String,
    /// The display size now, if known.
    pub current: Option<FrameSize>,
    /// Why this host cannot by itself; `None`: it can.
    pub unsupported: Option<String>,
    /// The driver setup (stage 2) can lift `unsupported` or `limit` here.
    pub setup_possible: bool,
    /// The compositor's portal makes the display in the host process
    /// (Wayland's virtual source in the viewer's size); no modes here.
    pub portal: bool,
}

/// The driver setup's state (stage 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupProgress {
    /// The elevated step runs or waits for the approval.
    Running,
    /// Done; it takes effect after sign-out or restart.
    AfterRestart,
    /// Done and the display manager restarted.
    Done,
    /// The step failed or was declined (reason).
    Failed(String),
}

/// The platform side of the virtual display.
pub trait VirtualDisplayDriver: Send + Sync {
    fn probe(&self) -> io::Result<DisplayProbe>;
    /// Sets the display to the fitted `size`; the size it has afterwards.
    fn apply(&self, size: FrameSize) -> io::Result<FrameSize>;
    /// The size `apply(size)` will set, when the driver takes only listed
    /// modes (Windows: the nearest one, `best_mode`); `None`: `size` itself
    /// (X11 RandR makes se's own mode). Blocks briefly: call it without
    /// the host's state lock.
    fn chosen_size(&self, _size: FrameSize) -> Option<FrameSize> {
        None
    }
    /// Starts the driver setup in the background unless this version tried
    /// already; its state now.
    fn request_setup(&self) -> SetupProgress;
    /// Gives a physical monitor its own mode back after the last session
    /// that asked for se's display there.
    fn restore(&self) -> io::Result<()> {
        Ok(())
    }
    /// Sets the physical monitor that shows the desktop to the mode it lists
    /// nearest to `size` (`windows_driver::best_mode`; no mode of se's); the
    /// size it shows afterwards (plan 2026-11-17 P9, `set_display.rs`).
    /// `restore` gives its own mode back. A platform without it refuses.
    fn physical_mode(&self, size: FrameSize) -> io::Result<FrameSize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "no listed modes of the physical monitor ({}x{})",
                size.width, size.height
            ),
        ))
    }
}

pub(super) fn unsupported(probe: &DisplayProbe, reason: &str) -> VirtualDisplayReport {
    VirtualDisplayReport::new(
        VirtualDisplayState::Unsupported,
        probe.current,
        &probe.driver,
    )
    .with_reason(reason)
}

/// What a viewer that asks would get, without changing anything: the
/// answer of a probe session (`se screen probe`, the MCP tool `screen`).
/// `live`: a viewer session may change its wish with `SET_DISPLAY` (P9).
pub fn capability(driver: Option<Arc<dyn VirtualDisplayDriver>>) -> Option<VirtualDisplayReport> {
    let mut report = probe_answer(driver?)?;
    report.live = true;
    Some(report)
}

fn probe_answer(driver: Arc<dyn VirtualDisplayDriver>) -> Option<VirtualDisplayReport> {
    let probe = match driver.probe() {
        Ok(probe) => probe,
        Err(error) => {
            return Some(
                VirtualDisplayReport::new(VirtualDisplayState::Unsupported, None, "")
                    .with_reason(&format!("probe_failed: {error}")),
            )
        }
    };
    let inactive = |reason: &str| {
        VirtualDisplayReport::new(VirtualDisplayState::Inactive, probe.current, &probe.driver)
            .with_reason(reason)
    };
    Some(match probe.unsupported.as_deref() {
        Some(reason) if !probe.setup_possible => unsupported(&probe, reason),
        _ if probe.physical_monitor => inactive(REASON_PHYSICAL),
        _ if probe.modes || probe.portal => inactive(REASON_AVAILABLE),
        _ if probe.setup_possible => inactive(REASON_SETUP),
        _ => unsupported(&probe, "no_display_modes"),
    })
}
