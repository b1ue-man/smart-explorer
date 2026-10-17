//! Start switches of the GUI program `smart_explorer` (contract V9, decision
//! H8 of `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`).
//!
//! One decision function covers every way `run_gui` is started. It runs after
//! the hooks that keep their own detection: `main.rs` (`--mount-host`) and the
//! first two checks of `run_gui` (local read helper, Exec supervisor). Order of
//! V9: the existing checks `--sync-guardian` (as the only argument) and
//! `--sync-daemon` (anywhere) unchanged; then the background kinds before the
//! update confirmation; after the updater steps Windows `--unregister`, then
//! the window kinds. A start switch of V9 counts only as the first argument
//! and then needs its exact form; the value of a switch never becomes the
//! local start path. `--drive-restore` is no start kind: it only adds the
//! start reason to `--sync-daemon` (S2) and is never a start path.
#![allow(dead_code)] // Staged: R2, R4, R5 and R9 wire the start kinds into `run_gui`.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;

pub(crate) const ACCESS_CONSENT: &str = "--access-consent";
pub(crate) const SCREEN_INDICATOR: &str = "--screen-indicator";
pub(crate) const SCREEN_HOST_WORKER: &str = "--screen-host-worker";
pub(crate) const SCREEN_VIEW: &str = "--screen-view";
pub(crate) const SCREEN_GRID: &str = "--screen-grid";
pub(crate) const DESKTOP_SHELL: &str = "--desktop-shell";
pub(crate) const DESKTOP_SURFACE: &str = "--desktop-surface";
pub(crate) const DESKTOP_JUMPLIST_CLEAR: &str = "--desktop-jumplist-clear";
pub(crate) const OPEN_LOCATION: &str = "--open-location";
pub(crate) const OPEN_PAGE: &str = "--open-page";
const ROOM: &str = "--room";
const MONITOR: &str = "--monitor";
const GNOME_MONITOR: &str = "--gnome-monitor";
const LAYER: &str = "--layer";
const SYNC_GUARDIAN: &str = "--sync-guardian";
const SYNC_DAEMON: &str = "--sync-daemon";
const DRIVE_RESTORE: &str = "--drive-restore";
const UPDATED: &str = "--updated";
const UNREGISTER: &str = "--unregister";

/// Switches followed by a value; such a value never becomes the start path.
const VALUE_SWITCHES: [&str; 7] = [
    ACCESS_CONSENT,
    SCREEN_VIEW,
    ROOM,
    MONITOR,
    GNOME_MONITOR,
    OPEN_LOCATION,
    OPEN_PAGE,
];

/// Upper bounds of a value; far above every value se itself passes.
const MAX_ID: usize = 256;
const MAX_LOCATOR: usize = 4096;
const MAX_MONITOR_KEY: usize = 64;
const MAX_TARGET_JSON: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    AccessConsent,
    ScreenIndicator,
    ScreenHostWorker,
    ScreenView,
    ScreenGrid,
    DesktopShell,
    DesktopSurface,
    DesktopJumplistClear,
    OpenLocation,
    OpenPage,
}

const KINDS: [(&str, Kind); 10] = [
    (ACCESS_CONSENT, Kind::AccessConsent),
    (SCREEN_INDICATOR, Kind::ScreenIndicator),
    (SCREEN_HOST_WORKER, Kind::ScreenHostWorker),
    (SCREEN_VIEW, Kind::ScreenView),
    (SCREEN_GRID, Kind::ScreenGrid),
    (DESKTOP_SHELL, Kind::DesktopShell),
    (DESKTOP_SURFACE, Kind::DesktopSurface),
    (DESKTOP_JUMPLIST_CLEAR, Kind::DesktopJumplistClear),
    (OPEN_LOCATION, Kind::OpenLocation),
    (OPEN_PAGE, Kind::OpenPage),
];

/// Every start kind of the GUI program (V9), exactly these eleven.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartRequest {
    /// S1 consent window, `--access-consent <ID>`.
    AccessConsent { id: String },
    /// S4 indicator of a running screen session, `--screen-indicator`.
    ScreenIndicator,
    /// S4 host process, `--screen-host-worker` as the only argument.
    ScreenHostWorker,
    /// S4 viewer, `--screen-view <JSON>`.
    ScreenView { target_json: String },
    /// S4 group view, `--screen-grid [--room <ID>]`.
    ScreenGrid { room: Option<String> },
    /// S5 shell, `--desktop-shell`.
    DesktopShell,
    /// S5 surface of one monitor or of all layer outputs.
    DesktopSurface { target: SurfaceTarget },
    /// S5 Windows jump list removal from the GUI image.
    DesktopJumplistClear,
    /// S5 window at a location, `--open-location <LOCATOR>`.
    OpenLocation { locator: String },
    /// S5 window at a page, `--open-page <SEITE>`.
    OpenPage { page: StartPage },
    /// The explorer window, optionally at a local start path (existing).
    Gui { start_path: Option<PathBuf> },
}

/// `--desktop-surface --monitor <SCHLÜSSEL> | --gnome-monitor <INDEX> | --layer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceTarget {
    Monitor { key: String },
    GnomeMonitor { index: u32 },
    Layer,
}

/// Pages of `--open-page` (DSK-M3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartPage {
    ShareDevices,
    ShareRooms,
    ShareDevice { contact_id: String },
    ShareRoom { room_id: String },
    SettingsIntegration,
}

/// Where `run_gui` goes, in the order of V9.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartRoute {
    /// Existing: `--sync-guardian` as the only argument.
    SyncGuardian,
    /// Existing: `--sync-daemon` anywhere; `--drive-restore` names the S2
    /// start reason.
    SyncDaemon { drive_restore: bool },
    /// A background kind, before the update confirmation.
    Background(StartRequest),
    /// Existing (Windows): `--unregister` anywhere, after the updater steps.
    Unregister { updated: bool },
    /// A window kind after the updater steps; `updated` is `--updated`.
    Window {
        updated: bool,
        request: StartRequest,
    },
}

/// Platform facts the caller passes in (this file has no platform branches).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StartFacts {
    /// `--unregister` removes the shell registrations (Windows).
    pub(crate) unregister_supported: bool,
}

/// A malformed start of a V9 kind (`run_gui` reports it and starts nothing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartArgError {
    pub(crate) switch: &'static str,
    pub(crate) problem: ArgProblem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ArgProblem {
    MissingValue,
    NotUnicode,
    Invalid(String),
    Unexpected(String),
}

impl fmt::Display for StartArgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let switch = self.switch;
        match &self.problem {
            ArgProblem::MissingValue => write!(formatter, "{switch} braucht einen Wert"),
            ArgProblem::NotUnicode => write!(formatter, "Wert von {switch} ist kein Unicode"),
            ArgProblem::Invalid(why) => write!(formatter, "Ungültiger Wert für {switch}: {why}"),
            ArgProblem::Unexpected(arg) => write!(formatter, "Unerwartet nach {switch}: {arg}"),
        }
    }
}

impl std::error::Error for StartArgError {}

impl StartRequest {
    /// Started before the update confirmation (V9).
    pub(crate) fn is_background(&self) -> bool {
        matches!(
            self,
            Self::DesktopShell
                | Self::DesktopSurface { .. }
                | Self::DesktopJumplistClear
                | Self::ScreenHostWorker
                | Self::ScreenIndicator
        )
    }

    /// The exact arguments that start this kind; [`parse`] reads them back.
    /// A start path that begins with `--` cannot be expressed (as before).
    pub(crate) fn to_args(&self) -> Vec<OsString> {
        match self {
            Self::AccessConsent { id } => vec![ACCESS_CONSENT.into(), id.into()],
            Self::ScreenIndicator => vec![SCREEN_INDICATOR.into()],
            Self::ScreenHostWorker => vec![SCREEN_HOST_WORKER.into()],
            Self::ScreenView { target_json } => vec![SCREEN_VIEW.into(), target_json.into()],
            Self::ScreenGrid { room: None } => vec![SCREEN_GRID.into()],
            Self::ScreenGrid { room: Some(room) } => {
                vec![SCREEN_GRID.into(), ROOM.into(), room.into()]
            }
            Self::DesktopShell => vec![DESKTOP_SHELL.into()],
            Self::DesktopSurface { target } => match target {
                SurfaceTarget::Monitor { key } => {
                    vec![DESKTOP_SURFACE.into(), MONITOR.into(), key.into()]
                }
                SurfaceTarget::GnomeMonitor { index } => vec![
                    DESKTOP_SURFACE.into(),
                    GNOME_MONITOR.into(),
                    index.to_string().into(),
                ],
                SurfaceTarget::Layer => vec![DESKTOP_SURFACE.into(), LAYER.into()],
            },
            Self::DesktopJumplistClear => vec![DESKTOP_JUMPLIST_CLEAR.into()],
            Self::OpenLocation { locator } => vec![OPEN_LOCATION.into(), locator.into()],
            Self::OpenPage { page } => vec![OPEN_PAGE.into(), page.as_arg().into()],
            Self::Gui { start_path } => start_path.iter().map(|path| path.into()).collect(),
        }
    }
}

impl StartPage {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        if let Some(id) = value.strip_prefix("share-device:") {
            return check_id(id).map(|contact_id| Self::ShareDevice { contact_id });
        }
        if let Some(id) = value.strip_prefix("share-room:") {
            return check_id(id).map(|room_id| Self::ShareRoom { room_id });
        }
        match value {
            "share-devices" => Ok(Self::ShareDevices),
            "share-rooms" => Ok(Self::ShareRooms),
            "settings-integration" => Ok(Self::SettingsIntegration),
            _ => Err(format!("unbekannte Seite {value:?}")),
        }
    }

    pub(crate) fn as_arg(&self) -> String {
        match self {
            Self::ShareDevices => "share-devices".into(),
            Self::ShareRooms => "share-rooms".into(),
            Self::ShareDevice { contact_id } => format!("share-device:{contact_id}"),
            Self::ShareRoom { room_id } => format!("share-room:{room_id}"),
            Self::SettingsIntegration => "settings-integration".into(),
        }
    }
}

/// Whether `arg` is the first-position switch of a V9 start kind.
pub(crate) fn is_start_switch(arg: &OsStr) -> bool {
    kind_of(arg).is_some()
}

/// The route of `run_gui` for its arguments without the program name.
pub(crate) fn route(args: &[OsString], facts: StartFacts) -> Result<StartRoute, StartArgError> {
    if args.len() == 1 && args[0].as_os_str() == OsStr::new(SYNC_GUARDIAN) {
        return Ok(StartRoute::SyncGuardian);
    }
    if has(args, SYNC_DAEMON) {
        return Ok(StartRoute::SyncDaemon {
            drive_restore: has(args, DRIVE_RESTORE),
        });
    }
    let request = parse(args)?;
    if request.is_background() {
        return Ok(StartRoute::Background(request));
    }
    let updated = has(args, UPDATED);
    if facts.unregister_supported && has(args, UNREGISTER) {
        return Ok(StartRoute::Unregister { updated });
    }
    Ok(StartRoute::Window { updated, request })
}

/// The start kind of the arguments without the program name.
pub(crate) fn parse(args: &[OsString]) -> Result<StartRequest, StartArgError> {
    let Some((switch, kind)) = args.first().and_then(|first| kind_of(first.as_os_str())) else {
        return Ok(StartRequest::Gui {
            start_path: start_path(args),
        });
    };
    let mut rest = Values {
        switch,
        items: args[1..].iter(),
    };
    let request = match kind {
        Kind::AccessConsent => StartRequest::AccessConsent {
            id: rest.value_with(check_id)?,
        },
        Kind::ScreenIndicator => StartRequest::ScreenIndicator,
        Kind::ScreenHostWorker => StartRequest::ScreenHostWorker,
        Kind::ScreenView => StartRequest::ScreenView {
            target_json: rest.value_with(check_target_json)?,
        },
        Kind::ScreenGrid => StartRequest::ScreenGrid {
            room: rest.optional_room()?,
        },
        Kind::DesktopShell => StartRequest::DesktopShell,
        Kind::DesktopSurface => StartRequest::DesktopSurface {
            target: rest.surface_target()?,
        },
        Kind::DesktopJumplistClear => StartRequest::DesktopJumplistClear,
        Kind::OpenLocation => StartRequest::OpenLocation {
            locator: rest.value_with(check_locator)?,
        },
        Kind::OpenPage => StartRequest::OpenPage {
            page: rest.value_with(StartPage::parse)?,
        },
    };
    rest.finish()?;
    Ok(request)
}

fn kind_of(arg: &OsStr) -> Option<(&'static str, Kind)> {
    KINDS
        .iter()
        .copied()
        .find(|(switch, _)| arg == OsStr::new(switch))
}

fn has(args: &[OsString], switch: &str) -> bool {
    args.iter().any(|arg| arg.as_os_str() == OsStr::new(switch))
}

/// Existing rule (`N/lib.rs`): the first argument that does not start with
/// `--`; new in V9: the value of a value switch is skipped.
fn start_path(args: &[OsString]) -> Option<PathBuf> {
    let mut skip_value = false;
    for arg in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        let lossy = arg.to_string_lossy();
        let text: &str = &lossy;
        if text.starts_with("--") {
            skip_value = VALUE_SWITCHES.contains(&text);
            continue;
        }
        return Some(PathBuf::from(arg));
    }
    None
}

/// The arguments after the switch of a start kind.
struct Values<'a> {
    switch: &'static str,
    items: std::slice::Iter<'a, OsString>,
}

impl Values<'_> {
    fn error(&self, problem: ArgProblem) -> StartArgError {
        StartArgError {
            switch: self.switch,
            problem,
        }
    }

    fn unexpected(&self, argument: &OsStr) -> StartArgError {
        self.error(ArgProblem::Unexpected(
            argument.to_string_lossy().into_owned(),
        ))
    }

    fn text(&mut self) -> Result<String, StartArgError> {
        let item = self.items.next();
        let item = item.ok_or_else(|| self.error(ArgProblem::MissingValue))?;
        let text = item
            .to_str()
            .ok_or_else(|| self.error(ArgProblem::NotUnicode))?;
        if text.starts_with("--") {
            return Err(self.error(ArgProblem::MissingValue));
        }
        Ok(text.to_string())
    }

    fn value_with<T>(
        &mut self,
        check: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<T, StartArgError> {
        let text = self.text()?;
        check(&text).map_err(|reason| self.error(ArgProblem::Invalid(reason)))
    }

    fn optional_room(&mut self) -> Result<Option<String>, StartArgError> {
        match self.items.next() {
            None => Ok(None),
            Some(flag) if flag.as_os_str() == OsStr::new(ROOM) => {
                self.value_with(check_id).map(Some)
            }
            Some(other) => Err(self.unexpected(other)),
        }
    }

    fn surface_target(&mut self) -> Result<SurfaceTarget, StartArgError> {
        let flag = self.items.next();
        match flag.and_then(|flag| flag.to_str()) {
            Some(MONITOR) => Ok(SurfaceTarget::Monitor {
                key: self.value_with(check_monitor_key)?,
            }),
            Some(GNOME_MONITOR) => Ok(SurfaceTarget::GnomeMonitor {
                index: self.value_with(|value| value.parse::<u32>().map_err(|e| e.to_string()))?,
            }),
            Some(LAYER) => Ok(SurfaceTarget::Layer),
            _ => Err(self.error(ArgProblem::Invalid(
                "erwartet --monitor <SCHLÜSSEL>, --gnome-monitor <INDEX> oder --layer".into(),
            ))),
        }
    }

    fn finish(mut self) -> Result<(), StartArgError> {
        match self.items.next() {
            None => Ok(()),
            Some(extra) => Err(self.unexpected(extra)),
        }
    }
}

/// IDs of approvals, contacts and rooms: 1 to 256 bytes without blanks or
/// control characters.
fn check_id(value: &str) -> Result<String, String> {
    let blank = |ch: char| ch.is_control() || ch.is_whitespace();
    if value.is_empty() || value.len() > MAX_ID || value.chars().any(blank) {
        return Err(format!(
            "Kennung {value:?} (1 bis {MAX_ID} Byte, ohne Leerzeichen)"
        ));
    }
    Ok(value.to_string())
}

fn check_locator(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > MAX_LOCATOR || value.chars().any(char::is_control) {
        return Err(format!(
            "Ort mit 1 bis {MAX_LOCATOR} Byte ohne Steuerzeichen erwartet"
        ));
    }
    Ok(value.to_string())
}

/// Windows key (16 hex characters), RandR output name; also a file name stem.
fn check_monitor_key(value: &str) -> Result<String, String> {
    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-');
    let shape = !value.is_empty() && value.len() <= MAX_MONITOR_KEY && !value.starts_with('.');
    if !shape || !value.bytes().all(allowed) {
        return Err(format!("Monitorschlüssel {value:?} (A-Z, a-z, 0-9, . _ -)"));
    }
    Ok(value.to_string())
}

/// The S4 viewer target is read by S4 itself; here only its outer form.
fn check_target_json(value: &str) -> Result<String, String> {
    if value.len() > MAX_TARGET_JSON {
        return Err(format!("Ziel länger als {MAX_TARGET_JSON} Byte"));
    }
    match serde_json::from_str::<serde_json::Value>(value) {
        Ok(serde_json::Value::Object(_)) => Ok(value.to_string()),
        Ok(_) => Err("Ziel ist kein JSON-Objekt".into()),
        Err(error) => Err(format!("Ziel ist kein JSON: {error}")),
    }
}

#[cfg(test)]
#[path = "start_request_tests.rs"]
mod tests;
