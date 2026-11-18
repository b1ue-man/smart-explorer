//! `SET_DISPLAY` (plan 2026-11-17 P9, after P5): „Monitor“ and „Auflösung“
//! of the viewer within a running session.
//!
//! Before, the host took the viewer's wish only with the hello
//! (`ScreenClientHello.virtual_display`, `virtual_display_size`), so every
//! change closed the session and opened a new one. With this frame:
//!
//! - The host names the capability in its answer: `live: true` in
//!   `ScreenHelloOk.virtual_display` (`VirtualDisplayReport::live`). Only a
//!   session that sent a wish and that the host's display driver serves gets
//!   that answer; a host of v0.5.198 or older never sets it, and an older
//!   viewer ignores it. A viewer sends `SET_DISPLAY` only then
//!   (`viewer_client_tasks.rs`, `negotiated`): a host without it ends the
//!   session on an unknown frame. Without it the viewer opens the session
//!   again, as before.
//! - The payload is the viewer's `VirtualDisplayChoice` as JSON (the wish
//!   and the fixed size; no size follows the window).
//! - The host decides as at the opening (`virtual_display_host::switch`):
//!   it releases the session's claim, gives a physical monitor its own mode
//!   back when the session no longer asks for se's display there, creates or
//!   attaches the virtual display in the size, or sets a size the physical
//!   monitor lists (`physical_size`); then it prepares the session's monitor
//!   source again (`display_source`), so only capture and encoder open anew
//!   with a key frame. The new state reaches the viewer in
//!   `HostStats.chain.virtual_display`.
use super::pixel_convert::FrameSize;
use super::virtual_display::VirtualDisplayWish;
use super::window_list::{MonitorInfo, SourceId};
use super::wire_hello::ScreenHelloOk;

/// Viewer to host: JSON `VirtualDisplayChoice`. The next free tag after
/// `FRAME_TIMING` (52).
pub const SET_DISPLAY: u8 = 53;
/// Largest payload of `SET_DISPLAY` (a wish and a size as JSON).
pub const MAX_SET_DISPLAY_BYTES: usize = 256;

/// Whether the viewer may send `SET_DISPLAY` in the session `hello` answered.
pub fn negotiated(hello: Option<&ScreenHelloOk>) -> bool {
    hello
        .and_then(|hello| hello.virtual_display.as_ref())
        .is_some_and(|report| report.live)
}

/// What the display shows after a change, for the choice of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// se's virtual display (the session asked for it and it is active).
    Virtual,
    /// The physical monitor(s) as they are, or in a listed mode.
    Physical,
}

/// The monitor a session shows after „Monitor“ changed: with se's display
/// the session's monitor when it is virtual, else the first virtual one (a
/// Windows host attaches its virtual monitor beside the physical ones),
/// else the session's monitor; without it the session's monitor when it is
/// a physical one that still exists, else the primary physical monitor,
/// else the first physical one. `current` is the id of the session's
/// monitor (`monitor:<id>` without the prefix). `None` without monitors.
pub fn display_source(
    monitors: &[MonitorInfo],
    current: Option<&str>,
    shown: Shown,
) -> Option<SourceId> {
    let known = |id: &str| monitors.iter().find(|monitor| monitor.id == id);
    let current = current.and_then(known);
    let chosen = match shown {
        Shown::Virtual => current
            .filter(|monitor| monitor.is_virtual)
            .or_else(|| monitors.iter().find(|monitor| monitor.is_virtual))
            .or(current),
        Shown::Physical => current
            .filter(|monitor| !monitor.is_virtual)
            .or_else(|| {
                monitors
                    .iter()
                    .find(|monitor| monitor.primary && !monitor.is_virtual)
            })
            .or_else(|| monitors.iter().find(|monitor| !monitor.is_virtual)),
    }
    .or_else(|| monitors.iter().find(|monitor| monitor.primary))
    .or_else(|| monitors.first())?;
    Some(SourceId::Monitor(chosen.id.clone()))
}

/// The size a physical monitor is set to: only a fixed size („Auflösung“
/// other than „Fenstergröße folgen“), only where the session shows the
/// physical monitor (`auto` at a monitor, `off`), and only on a host with a
/// physical monitor. A window size never changes a physical monitor.
pub fn physical_size(
    wish: VirtualDisplayWish,
    physical_monitor: bool,
    fixed: Option<FrameSize>,
) -> Option<FrameSize> {
    let shows_physical = match wish {
        VirtualDisplayWish::Off => true,
        VirtualDisplayWish::Auto => physical_monitor,
        VirtualDisplayWish::Request => false,
    };
    fixed.filter(|_| physical_monitor && shows_physical)
}

#[cfg(test)]
#[path = "set_display_task_tests.rs"]
mod set_display_task_tests;
