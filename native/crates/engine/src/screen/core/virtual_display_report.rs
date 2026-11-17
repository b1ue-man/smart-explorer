//! What the host did with the viewer's wish (`ScreenHelloOk.virtual_display`,
//! `HostStats.chain.virtual_display`, plan 2026-11-17 P5): its state, the
//! display size, the driver and why; the texts of the viewers and of `se`.
use serde::{Deserialize, Serialize};

use super::super::pixel_convert::FrameSize;
use super::{size_text, LIMIT_UNSUPPORTED_PREFIX};

/// Reasons of `Inactive` a host names.
pub const REASON_OFF: &str = "off";
pub const REASON_PHYSICAL: &str = "physical_monitor";
/// A probe: the display follows a viewer that asks (`randr`, `dummy`).
pub const REASON_AVAILABLE: &str = "available";
/// A probe: the driver setup with one approval comes first.
pub const REASON_SETUP: &str = "setup_needed";

/// The state of the virtual display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualDisplayState {
    /// Not used: a physical monitor, the wish `off`, a probe.
    Inactive,
    /// The display follows the viewer (or has its fixed size).
    Active,
    /// The compositor decides (Wayland): se offered the portal's virtual
    /// source in the viewer's size.
    Offered,
    /// The driver setup waits for the approval of the operating system.
    Setup,
    /// The driver is set up; it takes effect after sign-out or restart.
    AfterRestart,
    /// The host cannot; `reason` says why.
    Unsupported,
    /// A state of a newer host.
    #[serde(other)]
    Other,
}

/// `ScreenHelloOk.virtual_display`: state, the display size now and the
/// driver (`randr`, `dummy`, `bochs`, `portal`, `windows`, …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDisplayReport {
    pub state: VirtualDisplayState,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub driver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl VirtualDisplayReport {
    pub fn new(state: VirtualDisplayState, size: Option<FrameSize>, driver: &str) -> Self {
        Self {
            state,
            width: size.map_or(0, |size| size.width),
            height: size.map_or(0, |size| size.height),
            driver: driver.into(),
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// The display size, if the report names one.
    pub fn size(&self) -> Option<FrameSize> {
        (self.width > 0 && self.height > 0).then_some(FrameSize::new(self.width, self.height))
    }

    /// `virtual_display_unsupported:<reason>` for the limits of `HELLO_OK`.
    pub fn limit(&self) -> Option<String> {
        (self.state == VirtualDisplayState::Unsupported).then(|| {
            format!(
                "{LIMIT_UNSUPPORTED_PREFIX}{}",
                self.reason.as_deref().unwrap_or("unknown")
            )
        })
    }

    /// The German line of the viewers and of `se screen probe`.
    pub fn text(&self) -> String {
        let size = self.size().map(size_text).unwrap_or_else(|| "?".into());
        let reason = self.reason.as_deref();
        let text = match self.state {
            VirtualDisplayState::Inactive => match reason {
                Some(REASON_PHYSICAL) => "aus (Monitor am Gerät)".to_string(),
                Some(REASON_AVAILABLE) => format!("bereit ({})", self.driver),
                Some(REASON_SETUP) => "bereit nach einmaliger Einrichtung mit Freigabe".into(),
                _ => "aus".to_string(),
            },
            VirtualDisplayState::Active => format!("{size} ({})", self.driver),
            VirtualDisplayState::Offered => {
                format!("{size} am Portal angeboten, die Sitzung entscheidet")
            }
            VirtualDisplayState::Setup => {
                "wird eingerichtet (Freigabe des Betriebssystems)".to_string()
            }
            VirtualDisplayState::AfterRestart => {
                "eingerichtet, wirkt nach Abmeldung oder Neustart".to_string()
            }
            VirtualDisplayState::Unsupported => {
                format!("nicht möglich ({})", reason.unwrap_or("unbekannt"))
            }
            VirtualDisplayState::Other => "unbekannter Zustand".to_string(),
        };
        format!("Virtueller Bildschirm: {text}")
    }

    /// The short form of the statistics (`HostStats.chain.virtual_display`)
    /// and of `se screen probe`: `active:1920x1080:randr`, `inactive:off`, …
    pub fn chain_key(&self) -> String {
        let reason = self.reason.as_deref().unwrap_or("-");
        match self.state {
            VirtualDisplayState::Active => {
                format!("active:{}x{}:{}", self.width, self.height, self.driver)
            }
            VirtualDisplayState::Offered => {
                format!("offered:{}x{}:{}", self.width, self.height, self.driver)
            }
            VirtualDisplayState::Unsupported => format!("unsupported:{reason}"),
            VirtualDisplayState::Inactive => format!("inactive:{reason}"),
            VirtualDisplayState::Setup => "setup".into(),
            VirtualDisplayState::AfterRestart => "after_restart".into(),
            VirtualDisplayState::Other => "other".into(),
        }
    }
}
