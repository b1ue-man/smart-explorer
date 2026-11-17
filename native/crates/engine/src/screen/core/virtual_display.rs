//! Virtual display in the viewer's resolution (plan 2026-11-17 P5, TODO
//! VIRTUELLER-BILDSCHIRM; user 2026-11-17: „ja“, and „ich will per wahl auch
//! auf den eigenen monitor wechseln können, damit ich die auflösung
//! einstellen kann“). A host without a physical monitor shows its desktop in
//! the size of the viewer: the display size of the hello first, then every
//! size of `SET_MODE`; a viewer may also ask for it at a physical monitor
//! (`request`) and fix the size (`virtual_display_size`). This module holds
//! the platform-neutral rules: the viewer's wish, the facts that tell a
//! physical monitor, when a session may change the display and se's mode
//! names. Its parts: fitting a size to the driver and the mode timings
//! (`virtual_display_fit.rs`), the answer of the host
//! (`virtual_display_report.rs`), which session decides
//! (`virtual_display_owner.rs`). The platforms apply it through
//! `VirtualDisplayDriver` (`screen/os/virtual_display_os.rs`);
//! `virtual_display_host.rs` runs it.
use std::io;

use serde::{Deserialize, Serialize};

use super::pixel_convert::FrameSize;
use super::wire_hello::{SessionMode, MAX_DISPLAY_SIDE};

/// `ScreenServerHello.virtual_display`: the host reads the viewer's wish and
/// answers with `ScreenHelloOk.virtual_display`.
pub const VIRTUAL_DISPLAY_V1: u16 = 1;
/// Prefix of the display modes se creates (`SE_1920x1080`).
pub const MODE_PREFIX: &str = "SE_";
/// Shortest time between two changes of the display size.
pub const MIN_RESIZE_GAP_MS: u64 = 1000;
/// Display widths are a multiple of this (CVT cell granularity, 8 pixels).
pub const WIDTH_STEP: u32 = 8;
/// Smallest display se sets: a tiny viewer window does not shrink the
/// desktop below a usable size (assumption, not measured).
pub const MIN_WIDTH: u32 = 640;
pub const MIN_HEIGHT: u32 = 360;
/// Refresh rate of se's modes.
pub const REFRESH_HZ: u32 = 60;
/// Limit text of a host without a virtual display (`HELLO_OK` limits).
pub const LIMIT_UNSUPPORTED_PREFIX: &str = "virtual_display_unsupported:";
/// Reason of a Windows host until its display driver is decided (P5, E6).
pub const WINDOWS_DRIVER_PENDING: &str = "windows_driver_pending";

/// The sizes of the viewer's menu „Auflösung“; every width is a multiple of
/// `WIDTH_STEP`, so each one is set exactly.
pub const SIZE_CHOICES: [FrameSize; 13] = [
    FrameSize::new(1280, 720),
    FrameSize::new(1280, 800),
    FrameSize::new(1360, 768),
    FrameSize::new(1440, 900),
    FrameSize::new(1600, 900),
    FrameSize::new(1680, 1050),
    FrameSize::new(1920, 1080),
    FrameSize::new(1920, 1200),
    FrameSize::new(2560, 1080),
    FrameSize::new(2560, 1440),
    FrameSize::new(2560, 1600),
    FrameSize::new(3440, 1440),
    FrameSize::new(3840, 2160),
];

/// What the viewer wants (`ScreenClientHello.virtual_display`). A viewer
/// without the field (v0.5.197 and older) keeps the old behaviour: the host
/// never changes its display for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualDisplayWish {
    /// Follow the viewer when the host has no physical monitor.
    #[default]
    Auto,
    /// se's own display also at a physical monitor (display modes only; the
    /// driver setup never replaces a physical monitor).
    Request,
    /// The physical monitor as it is: never change the host's display; an
    /// unknown future value means this (`serde(other)` must be the last
    /// variant).
    #[serde(other)]
    Off,
}

impl VirtualDisplayWish {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Off, Self::Request];

    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
            Self::Request => "request",
        }
    }

    /// `auto`, `off` or `request` (CLI, settings, MCP).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "off" => Some(Self::Off),
            "request" => Some(Self::Request),
            _ => None,
        }
    }

    /// The German label of the viewer's menu „Monitor“.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatisch (eigener nur ohne Monitor am Gerät)",
            Self::Off => "Physischer Monitor",
            Self::Request => "Eigener (virtuell)",
        }
    }
}

/// The viewer's choice as the desktop viewer stores it and the requests
/// carry it (`StreamRequest`, `ViewerRequest`): the wish and a fixed size;
/// without a size the display follows the viewer's window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VirtualDisplayChoice {
    #[serde(default)]
    pub wish: VirtualDisplayWish,
    /// „Auflösung“: this size instead of the window's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<FrameSize>,
}

impl VirtualDisplayChoice {
    /// A fixed size within the hello's bounds (1..=16384 per side).
    pub fn validate(&self) -> io::Result<()> {
        match self.size {
            Some(size) if !side_valid(size) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "virtual display size is outside 1..=16384",
            )),
            _ => Ok(()),
        }
    }

    /// `auto`, `request 2560×1440`, … (CLI and the viewer's menu).
    pub fn text(&self) -> String {
        match self.size {
            Some(size) => format!("{} {}", self.wish.key(), size_text(size)),
            None => self.wish.key().to_string(),
        }
    }
}

/// Whether both sides of `size` are within 1..=16384.
pub fn side_valid(size: FrameSize) -> bool {
    let side = 1..=MAX_DISPLAY_SIDE;
    side.contains(&size.width) && side.contains(&size.height)
}

/// `1920x1080` (also `×` or `X`); `None` for anything else or a side outside
/// 1..=16384.
pub fn parse_size(text: &str) -> Option<FrameSize> {
    let text = text.trim();
    let (width, height) = text
        .split_once('x')
        .or_else(|| text.split_once('X'))
        .or_else(|| text.split_once('×'))?;
    let size = FrameSize::new(width.trim().parse().ok()?, height.trim().parse().ok()?);
    side_valid(size).then_some(size)
}

/// `1920×1080`.
pub fn size_text(size: FrameSize) -> String {
    format!("{}×{}", size.width, size.height)
}

/// One DRM connector (`/sys/class/drm/card<N>-<type>-<n>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectorFact {
    /// The connector type of its name (`HDMI-A`, `DP`, `Virtual`, …).
    pub kind: String,
    /// `status` reads `connected`.
    pub connected: bool,
    /// Bytes of its `edid`.
    pub edid_bytes: usize,
}

/// What tells a physical monitor (E1): a connected DRM connector whose type
/// is not `Virtual` (nor `Writeback`) with a non-empty EDID. Without DRM
/// (`/dev/dri` missing, VESA/fbdev) there is no physical monitor by DRM;
/// `display_outputs` adds the X server's view for a driver that hides its
/// connectors from DRM (decision of this package, `physical_monitor`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhysicalFacts {
    pub connectors: Vec<ConnectorFact>,
    /// Names of the X server's connected outputs, if the host asked it.
    pub display_outputs: Vec<String>,
}

impl PhysicalFacts {
    /// A physical monitor: a DRM connector as E1 says; or, when DRM lists
    /// no connector at all, a connected X output that is neither virtual
    /// (`virtual_output_name`) nor the emulated `default` of VESA/fbdev
    /// (the proprietary NVIDIA driver without kernel modesetting shows its
    /// monitors only there): never change a real monitor automatically.
    pub fn physical_monitor(&self) -> bool {
        let by_drm = self.connectors.iter().any(|connector| {
            connector.connected
                && connector.edid_bytes > 0
                && !connector.kind.eq_ignore_ascii_case("Virtual")
                && !connector.kind.eq_ignore_ascii_case("Writeback")
        });
        let blind_drm = self.connectors.is_empty();
        let by_display = blind_drm
            && self
                .display_outputs
                .iter()
                .any(|name| name != "default" && !virtual_output_name(name));
        by_drm || by_display
    }
}

/// Whether an X output name is a display without a screen: the outputs of
/// the Xorg driver `dummy` (`DUMMY0`…) and the virtual connectors of VM
/// graphics under `modesetting` (`Virtual-1`, `Virtual1`).
pub fn virtual_output_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("dummy") || lower.starts_with("virtual")
}

/// The connector type of a DRM connector directory name: `card0-HDMI-A-1`
/// → `HDMI-A`, `card1-Virtual-1` → `Virtual`; `None` for a card itself
/// (`card0`) or another name.
pub fn connector_kind(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("card")?;
    let (card, rest) = rest.split_once('-')?;
    if card.is_empty() || !card.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (kind, index) = rest.rsplit_once('-')?;
    if kind.is_empty() || index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(kind)
}

/// Whether a session changes the display (E1): only a viewer that sent a
/// wish (new viewers), only for a view or control session that streams
/// (never a probe, a preview, a metadata session or `se screen bench`, whose
/// hellos carry no wish); `auto` only without a physical monitor.
pub fn needs_virtual(
    mode: SessionMode,
    metadata_only: bool,
    wish: Option<VirtualDisplayWish>,
    physical_monitor: bool,
) -> bool {
    if metadata_only || !matches!(mode, SessionMode::View | SessionMode::Control) {
        return false;
    }
    match wish {
        None | Some(VirtualDisplayWish::Off) => false,
        Some(VirtualDisplayWish::Auto) => !physical_monitor,
        Some(VirtualDisplayWish::Request) => true,
    }
}

/// Whether the driver setup with an elevation (stage 2) may run: never at a
/// physical monitor (it replaces the X server's driver), never against the
/// wish `off`.
pub fn setup_allowed(wish: Option<VirtualDisplayWish>, physical_monitor: bool) -> bool {
    !physical_monitor
        && matches!(
            wish,
            Some(VirtualDisplayWish::Auto | VirtualDisplayWish::Request)
        )
}

/// The name of se's mode of `size`: `SE_<w>x<h>`.
pub fn mode_name(size: FrameSize) -> String {
    format!("{MODE_PREFIX}{}x{}", size.width, size.height)
}

/// Whether a display mode is one of se's own (removed when replaced).
pub fn is_own_mode(name: &str) -> bool {
    name.strip_prefix(MODE_PREFIX)
        .and_then(|size| size.split_once('x'))
        .is_some_and(|(w, h)| {
            !w.is_empty()
                && !h.is_empty()
                && w.bytes().all(|byte| byte.is_ascii_digit())
                && h.bytes().all(|byte| byte.is_ascii_digit())
        })
}

#[path = "virtual_display_fit.rs"]
mod fit;
pub use fit::{cvt_rb, fit_mode, DriverLimit, ModeTiming};
#[path = "virtual_display_owner.rs"]
mod owner;
pub use owner::{owner, Claim, Due, Planner};
#[path = "virtual_display_report.rs"]
mod report;
pub use report::{
    VirtualDisplayReport, VirtualDisplayState, REASON_AVAILABLE, REASON_OFF, REASON_PHYSICAL,
    REASON_SETUP,
};

#[cfg(test)]
#[path = "virtual_display_contract_task_tests.rs"]
mod virtual_display_contract_task_tests;
#[cfg(test)]
#[path = "virtual_display_task_tests.rs"]
mod virtual_display_task_tests;
