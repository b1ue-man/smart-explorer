//! What the host did with the viewer's wish (`ScreenHelloOk.virtual_display`,
//! `HostStats.chain.virtual_display`, plan 2026-11-17 P5): its state, the
//! display size, the driver and why; the texts of the viewers and of `se`.
//!
//! User rule of 2026-11-18 („Wahl gilt“): a setting the viewer chose is
//! used exactly as chosen; where the host cannot, the report says what was
//! chosen, what the host set and why (`requested`, `adjusted`), and which
//! monitor a session shows when the host moved it (`source`).
use serde::{Deserialize, Serialize};

use super::super::pixel_convert::FrameSize;
use super::super::wire_hello::SessionMode;
use super::windows_driver::reason_text;
use super::{size_text, VirtualDisplayWish, LIMIT_UNSUPPORTED_PREFIX};

/// Reasons of `Inactive` a host names.
pub const REASON_OFF: &str = "off";
pub const REASON_PHYSICAL: &str = "physical_monitor";
/// A probe: the display follows a viewer that asks (`randr`, `dummy`).
pub const REASON_AVAILABLE: &str = "available";
/// A probe: the driver setup with one approval comes first.
pub const REASON_SETUP: &str = "setup_needed";
/// Reason of `Unsupported`: the driver setup would lift it, but only a
/// viewer that may steer the screen („Bildschirm steuern“) starts it.
pub const REASON_SETUP_NEEDS_CONTROL: &str = "setup_needs_control";
/// Reason of `Inactive` when the physical monitor lists no mode for the
/// fixed size or refused it.
pub const REASON_PHYSICAL_MODE_REFUSED: &str = "physical_mode_refused";
/// The viewer's source is a window (or a window family, the test picture):
/// the explicitly chosen source wins over the display wish, which only
/// applies to a monitor source (`Unsupported` for `request` or a fixed
/// size, `Inactive` for `auto`).
pub const REASON_SOURCE_NOT_MONITOR: &str = "source_not_monitor";

/// Why the host set another size than the viewer's fixed one (`adjusted`):
/// the rules of se's display modes (at least 640 × 360 with the aspect ratio
/// kept, the width a multiple of 8, at most 16384 per side and the pixels
/// se's capture takes, `fit_mode`).
pub const ADJUST_MODE_RULES: &str = "mode_rules";
/// The limit of the display driver (`DriverLimit`).
pub const ADJUST_DRIVER_LIMIT: &str = "driver_limit";
/// Another session decides the size of the display (control, else the
/// newest session).
pub const ADJUST_OTHER_SESSION: &str = "other_session";
/// The size follows after the shortest gap between two changes.
pub const ADJUST_PENDING: &str = "pending";
/// The driver set another size than se asked for, or refused it.
pub const ADJUST_DRIVER: &str = "driver_set_other";
/// The physical monitor lists no mode of the size: its nearest one.
pub const ADJUST_PHYSICAL_MODE: &str = "physical_nearest_mode";
/// The physical monitor refused the mode (`refused: <error>`).
pub const ADJUST_REFUSED: &str = "refused";

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
    /// `HELLO_OK` only: this host takes `SET_DISPLAY` in the session
    /// (`set_display.rs`); absent from hosts before P9 and in the
    /// statistics.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub live: bool,
    /// The viewer's fixed size („Auflösung“) when the host set another one
    /// or none; absent otherwise and from older hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<FrameSize>,
    /// Why the host set another size than `requested` (`ADJUST_*`; a
    /// refused physical mode: `refused: <error>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjusted: Option<String>,
    /// The source the session shows (`monitor:<id>`) when the host moved it
    /// to another monitor than the viewer's (se's virtual monitor beside the
    /// physical one); absent otherwise and from older hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The viewer's monitor before that move (`monitor:<id>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_chosen: Option<String>,
}

impl VirtualDisplayReport {
    pub fn new(state: VirtualDisplayState, size: Option<FrameSize>, driver: &str) -> Self {
        Self {
            state,
            width: size.map_or(0, |size| size.width),
            height: size.map_or(0, |size| size.height),
            driver: driver.into(),
            reason: None,
            live: false,
            requested: None,
            adjusted: None,
            source: None,
            source_chosen: None,
        }
    }

    pub fn with_reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// The viewer fixed `requested`: when the report names another size (or
    /// none), it keeps `requested` and `why` (`ADJUST_*`); an equal size
    /// leaves the report as it is.
    pub fn with_requested(mut self, requested: FrameSize, why: &str) -> Self {
        if self.size() != Some(requested) {
            self.requested = Some(requested);
            self.adjusted = Some(why.into());
        }
        self
    }

    /// The host shows `shown` (`monitor:<id>`) instead of `chosen`; an
    /// equal source leaves the report as it is.
    pub fn with_source(mut self, chosen: Option<&str>, shown: &str) -> Self {
        if chosen != Some(shown) {
            self.source = Some(shown.into());
            self.source_chosen = chosen.map(str::to_string);
        }
        self
    }

    /// The answer to a wish of a view or control session whose source is
    /// no monitor (a window, a window family, the test picture): the chosen
    /// source wins, the report says so. `request` or a fixed size cannot
    /// apply (`Unsupported`, `REASON_SOURCE_NOT_MONITOR`, the size as
    /// `requested`); `auto` decides nothing for it (`Inactive`, same
    /// reason); `off` without a size changes nothing (`Inactive`, `off`).
    /// `None` without a wish, for a metadata session and for a probe or a
    /// preview, as `virtual_display_host::start`.
    pub fn not_monitor(
        wish: Option<VirtualDisplayWish>,
        fixed: Option<FrameSize>,
        mode: SessionMode,
        metadata_only: bool,
    ) -> Option<Self> {
        let wish = wish?;
        if metadata_only || !matches!(mode, SessionMode::View | SessionMode::Control) {
            return None;
        }
        let (state, reason) = match (wish, fixed) {
            (VirtualDisplayWish::Off, None) => (VirtualDisplayState::Inactive, REASON_OFF),
            (VirtualDisplayWish::Auto, None) => {
                (VirtualDisplayState::Inactive, REASON_SOURCE_NOT_MONITOR)
            }
            _ => (VirtualDisplayState::Unsupported, REASON_SOURCE_NOT_MONITOR),
        };
        let report = Self::new(state, None, "").with_reason(reason);
        Some(match fixed {
            Some(fixed) => report.with_requested(fixed, REASON_SOURCE_NOT_MONITOR),
            None => report,
        })
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

    /// The German line of the viewers and of `se screen probe`: the state,
    /// the size wherever the report names one, a size the host set other
    /// than the chosen one with its reason, and a moved source.
    pub fn text(&self) -> String {
        let mut line = format!("Virtueller Bildschirm: {}", self.state_text());
        if let Some(requested) = self.requested {
            let set = self.size().map_or_else(|| "keine".to_string(), size_text);
            let why = self
                .adjusted
                .as_deref()
                .map_or_else(String::new, adjust_text);
            line.push_str(&format!(
                " · gewählt {}, eingestellt {set}, Grund: {why}",
                size_text(requested)
            ));
        }
        if let Some(shown) = &self.source {
            match &self.source_chosen {
                Some(chosen) => line.push_str(&format!(
                    " · Quelle: {chosen} → jetzt {shown} (der virtuelle Bildschirm ist ein eigener Monitor)"
                )),
                None => line.push_str(&format!(" · Quelle: {shown}")),
            }
        }
        line
    }

    /// The German state of `text` (after „Virtueller Bildschirm: “).
    fn state_text(&self) -> String {
        let size = self.size().map(size_text).unwrap_or_else(|| "?".into());
        let at = self
            .size()
            .map(|size| format!(", {}", size_text(size)))
            .unwrap_or_default();
        let reason = self.reason.as_deref();
        match self.state {
            VirtualDisplayState::Inactive => match reason {
                Some(REASON_PHYSICAL) => format!("aus (Monitor am Gerät{at})"),
                Some(REASON_OFF) => format!("aus (physischer Monitor gewählt{at})"),
                Some(REASON_AVAILABLE) => format!("bereit ({}{at})", self.driver),
                Some(REASON_SETUP) => "bereit nach einmaliger Einrichtung mit Freigabe".into(),
                Some(REASON_PHYSICAL_MODE_REFUSED) => {
                    format!("aus (der Monitor lehnte die gewählte Auflösung ab{at})")
                }
                Some(REASON_SOURCE_NOT_MONITOR) => format!("aus ({}{at})", source_not_monitor()),
                Some(other) => format!("aus ({other}{at})"),
                None => match self.size() {
                    Some(size) => format!("aus ({})", size_text(size)),
                    None => "aus".to_string(),
                },
            },
            VirtualDisplayState::Active => format!("{size} ({})", self.driver),
            VirtualDisplayState::Offered => {
                format!("{size} am Portal angeboten, die Sitzung entscheidet")
            }
            VirtualDisplayState::Setup => match reason.and_then(reason_text) {
                Some(text) => text,
                None => "wird eingerichtet (Freigabe des Betriebssystems)".to_string(),
            },
            VirtualDisplayState::AfterRestart => {
                "eingerichtet, wirkt nach Abmeldung oder Neustart".to_string()
            }
            VirtualDisplayState::Unsupported => {
                let why = match reason {
                    Some(REASON_SOURCE_NOT_MONITOR) => source_not_monitor().to_string(),
                    Some(REASON_SETUP_NEEDS_CONTROL) => SETUP_NEEDS_CONTROL_TEXT.to_string(),
                    Some(reason) => reason_text(reason).unwrap_or_else(|| reason.to_string()),
                    None => "unbekannt".to_string(),
                };
                format!("nicht möglich ({why})")
            }
            VirtualDisplayState::Other => "unbekannter Zustand".to_string(),
        }
    }

    /// The short form of the statistics (`HostStats.chain.virtual_display`)
    /// and of `se screen probe`: `active:1920x1080:randr`, `inactive:off`, …;
    /// `;requested=<w>x<h>;adjusted=<ADJUST_*>` when the host set another
    /// size than the viewer's fixed one, `;source=monitor:<id>` when it moved
    /// the session to another monitor (always the last part).
    pub fn chain_key(&self) -> String {
        let reason = self.reason.as_deref().unwrap_or("-");
        let mut key = match self.state {
            VirtualDisplayState::Active => {
                format!("active:{}x{}:{}", self.width, self.height, self.driver)
            }
            VirtualDisplayState::Offered => {
                format!("offered:{}x{}:{}", self.width, self.height, self.driver)
            }
            VirtualDisplayState::Unsupported => format!("unsupported:{reason}"),
            VirtualDisplayState::Inactive => format!("inactive:{reason}"),
            // A setup names its reason where it has one (Windows at use time).
            VirtualDisplayState::Setup => match &self.reason {
                Some(reason) => format!("setup:{reason}"),
                None => "setup".into(),
            },
            VirtualDisplayState::AfterRestart => "after_restart".into(),
            VirtualDisplayState::Other => "other".into(),
        };
        if let Some(requested) = self.requested {
            let why = self.adjusted.as_deref().unwrap_or("-");
            let why = why.split(':').next().unwrap_or(why);
            key.push_str(&format!(
                ";requested={}x{};adjusted={why}",
                requested.width, requested.height
            ));
        }
        if let Some(shown) = &self.source {
            key.push_str(&format!(";source={shown}"));
        }
        key
    }
}

/// The German state of a statistics key (`chain_key`) that names the driver
/// setup or why the host cannot (`setup`, `setup:<reason>`,
/// `after_restart`, `unsupported:<reason>`), as `text` names the state;
/// `None` for the other keys, which the viewers show as they are. The parts
/// after `;` are left out.
pub fn chain_text(key: &str) -> Option<String> {
    let key = key.split(';').next().unwrap_or(key);
    let (state, reason) = match key.split_once(':') {
        Some((state, reason)) => (state, Some(reason)),
        None => (key, None),
    };
    let state = match state {
        "setup" => VirtualDisplayState::Setup,
        "after_restart" => VirtualDisplayState::AfterRestart,
        "unsupported" => VirtualDisplayState::Unsupported,
        _ => return None,
    };
    let mut report = VirtualDisplayReport::new(state, None, "");
    report.reason = reason.map(str::to_string);
    Some(report.state_text())
}

const SETUP_NEEDS_CONTROL_TEXT: &str =
    "die einmalige Einrichtung startet nur ein Betrachter mit dem Recht „Bildschirm steuern“";

fn source_not_monitor() -> &'static str {
    "die gewählte Quelle ist kein Monitor; der Monitor-Wunsch gilt nur für eine \
Monitorquelle, die gewählte Quelle hat Vorrang"
}

/// The German reason of `adjusted`.
pub fn adjust_text(why: &str) -> String {
    let (key, detail) = why.split_once(':').unwrap_or((why, ""));
    let text = match key {
        ADJUST_MODE_RULES => {
            "Regeln der Anzeigemodi (mindestens 640×360 im selben \
Seitenverhältnis, Breite ein Vielfaches von 8, höchstens 16384 je Seite und die Pixelgrenze der \
Aufnahme)"
        }
        ADJUST_DRIVER_LIMIT => "Grenze des Anzeigetreibers",
        ADJUST_OTHER_SESSION => {
            "eine andere Sitzung bestimmt die Größe (die mit Steuerung, sonst die neueste)"
        }
        ADJUST_PENDING => "die Änderung folgt nach dem kürzesten Abstand zwischen zwei Änderungen",
        ADJUST_DRIVER => "der Anzeigetreiber stellte eine andere Größe ein oder lehnte sie ab",
        ADJUST_PHYSICAL_MODE => {
            "der Monitor kennt diese Größe nicht, sein nächstliegender Modus gilt"
        }
        ADJUST_REFUSED => "der Monitor lehnte den Modus ab",
        REASON_SOURCE_NOT_MONITOR => "die gewählte Quelle ist kein Monitor",
        other => other,
    };
    let detail = detail.trim();
    if detail.is_empty() {
        text.to_string()
    } else {
        format!("{text}: {detail}")
    }
}
