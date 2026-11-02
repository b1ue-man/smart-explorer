//! Wire-safe domain values; raw platform handles never identify a remote window.
use serde::{Deserialize, Serialize};

pub const FAMILY_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowIdentity {
    /// Fresh host-generated nonce for this authorized source lifetime.
    pub session: String,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeWindowId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowBinding {
    pub native: NativeWindowId,
    /// OS-window-lifetime marker, not just PID/title/class/handle.
    pub birth: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinPlatform {
    Windows,
    X11,
}

/// Internal authenticated daemon/worker transfer only. Never a peer request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPin {
    pub platform: PinPlatform,
    pub namespace: String,
    pub binding: WindowBinding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl CaptureGeometry {
    pub fn valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && i64::from(self.x) + i64::from(self.width) <= i64::from(i32::MAX)
            && i64::from(self.y) + i64::from(self.height) <= i64::from(i32::MAX)
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        self.valid()
            && x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    Main,
    Dialog,
    Popup,
    Menu,
    Tooltip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowOperations {
    pub resize: bool,
    pub focus: bool,
    pub close: bool,
    pub input: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyMember {
    pub identity: WindowIdentity,
    pub owner: Option<WindowIdentity>,
    pub app_id: Option<String>,
    pub app: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<crate::screen::core::app_icon::IconRef>,
    pub kind: WindowKind,
    pub geometry: CaptureGeometry,
    /// Smaller numbers are behind larger numbers; independent capture surfaces.
    pub z_order: u32,
    pub focused: bool,
    pub visible: bool,
    pub minimized: bool,
    pub operations: WindowOperations,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilySnapshot {
    pub version: u16,
    pub session: String,
    pub revision: u64,
    pub layout_revision: u64,
    pub root: WindowIdentity,
    pub members: Vec<FamilyMember>,
    /// Explicit reasons a surface cannot safely be associated or operated.
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyUpdate {
    pub previous_revision: u64,
    /// Always complete; a lost event requires no replay history.
    pub snapshot: FamilySnapshot,
    pub removed: Vec<WindowIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum WindowOperation {
    Resize { width: u32, height: u32 },
    Focus,
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInputTarget {
    pub identity: WindowIdentity,
    pub snapshot_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// Resize/close are asynchronous app/WM requests; the next snapshot is truth.
    Requested,
    FocusConfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedWindow {
    pub binding: WindowBinding,
    pub owner: Option<NativeWindowId>,
    /// Platform-verified process/session association, not a display label.
    pub process: String,
    pub app_id: Option<String>,
    pub app: String,
    pub title: String,
    pub kind: WindowKind,
    pub geometry: CaptureGeometry,
    pub z_order: u32,
    pub focused: bool,
    pub visible: bool,
    pub minimized: bool,
    pub operations: WindowOperations,
}

pub struct Observation {
    pub windows: Vec<ObservedWindow>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureMember {
    pub identity: WindowIdentity,
    pub binding: WindowBinding,
    pub geometry: CaptureGeometry,
    pub z_order: u32,
    pub input: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureBoundary {
    pub session: String,
    /// Capture layout epoch, independent of title/icon/focus metadata revisions.
    pub revision: u64,
    pub members: Vec<CaptureMember>,
}

impl CaptureBoundary {
    pub fn bounds(&self) -> Option<CaptureGeometry> {
        let left = self.members.iter().map(|m| m.geometry.x).min()?;
        let top = self.members.iter().map(|m| m.geometry.y).min()?;
        let right = self
            .members
            .iter()
            .map(|m| i64::from(m.geometry.x) + i64::from(m.geometry.width))
            .max()?;
        let bottom = self
            .members
            .iter()
            .map(|m| i64::from(m.geometry.y) + i64::from(m.geometry.height))
            .max()?;
        let bounds = CaptureGeometry {
            x: left,
            y: top,
            width: u32::try_from(right - i64::from(left)).ok()?,
            height: u32::try_from(bottom - i64::from(top)).ok()?,
        };
        bounds.valid().then_some(bounds)
    }

    /// Normalized family-frame coordinates become a concrete member and its
    /// normalized source coordinates. Blank canvas space cannot target the desktop.
    pub fn map_pointer(&self, x: u16, y: u16) -> Option<(&CaptureMember, u16, u16)> {
        let bounds = self.bounds()?;
        let px =
            i64::from(bounds.x) + i64::from(x) * i64::from(bounds.width.saturating_sub(1)) / 65_535;
        let py = i64::from(bounds.y)
            + i64::from(y) * i64::from(bounds.height.saturating_sub(1)) / 65_535;
        let px = i32::try_from(px).ok()?;
        let py = i32::try_from(py).ok()?;
        let member = self
            .members
            .iter()
            .filter(|m| m.input && m.geometry.contains(px, py))
            .max_by_key(|m| m.z_order)?;
        let along = |pixel: i32, start: i32, length: u32| -> u16 {
            let value = (i64::from(pixel) - i64::from(start)) * 65_535
                / i64::from(length.saturating_sub(1).max(1));
            value.clamp(0, 65_535) as u16
        };
        Some((
            member,
            along(px, member.geometry.x, member.geometry.width),
            along(py, member.geometry.y, member.geometry.height),
        ))
    }

    /// Blank space between family surfaces must stay blank, never desktop pixels.
    pub fn member_at(&self, x: i32, y: i32) -> Option<&CaptureMember> {
        self.members
            .iter()
            .filter(|m| m.geometry.contains(x, y))
            .max_by_key(|m| m.z_order)
    }
}
