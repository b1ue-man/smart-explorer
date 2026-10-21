//! What runs on the host (decision C5) and what a session can show: the
//! source ids `monitor:<id>`, `window:<id>` and `synthetic`, the monitors of
//! `HELLO_OK`, and the windows and apps of the `WINDOWS` frame with the list
//! source that produced them. The only bound of a `WINDOWS` list is its
//! frame: the payload may take `MAX_WINDOWS_BYTES` (1 MiB), the size every
//! viewer of protocol version 1 reads at most, so a foreign peer cannot make
//! a viewer allocate more. There is no count of entries (R15c); `bounded`
//! keeps as many entries as fit into the frame.
use std::fmt;
use std::io;

use serde::{Deserialize, Serialize};

use super::wire::{invalid, MAX_WINDOWS_BYTES};

/// Longest id of a source, window or monitor: native ids (HWND, X11 window,
/// Wayland handle, KDE UUID) are at most 36 characters.
pub(crate) const MAX_ID_BYTES: usize = 64;
const MAX_TITLE_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 64;
const MAX_LIMITS: usize = 16;

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && id.bytes().all(|byte| byte.is_ascii_graphic())
}

/// What a session shows. `None` in a hello or `SET_MODE` means the primary
/// monitor.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) enum SourceId {
    Monitor(String),
    Window(String),
    /// The test picture; no screen content (C7: allowed without indicator).
    Synthetic,
}

impl SourceId {
    /// `monitor:<id>`, `window:<id>` or `synthetic`; an id is 1 to 64
    /// visible ASCII characters.
    pub(crate) fn parse(text: &str) -> io::Result<Self> {
        match text.split_once(':') {
            _ if text == "synthetic" => Ok(Self::Synthetic),
            Some(("monitor", id)) if valid_id(id) => Ok(Self::Monitor(id.into())),
            Some(("window", id)) if valid_id(id) => Ok(Self::Window(id.into())),
            _ => Err(invalid(
                "screen source must be monitor:<id>, window:<id> or synthetic",
            )),
        }
    }

    /// Whether the source shows screen content (everything but `synthetic`).
    pub(crate) fn is_screen_content(&self) -> bool {
        !matches!(self, Self::Synthetic)
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Monitor(id) => write!(f, "monitor:{id}"),
            Self::Window(id) => write!(f, "window:{id}"),
            Self::Synthetic => f.write_str("synthetic"),
        }
    }
}

impl TryFrom<String> for SourceId {
    type Error = io::Error;

    fn try_from(text: String) -> io::Result<Self> {
        Self::parse(&text)
    }
}

impl From<SourceId> for String {
    fn from(source: SourceId) -> Self {
        source.to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MonitorInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WindowEntry {
    pub id: String,
    pub title: String,
    pub app: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub monitor: Option<String>,
    #[serde(default)]
    pub minimized: bool,
    #[serde(default)]
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AppEntry {
    pub app_id: String,
    pub name: String,
    #[serde(default)]
    pub pid: Option<u32>,
}

/// Which platform interface listed the windows or apps (C5, SCR-M4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ListSource {
    EnumWindows,
    Ewmh,
    X11QueryTree,
    ExtForeignToplevel,
    WlrForeignToplevel,
    KdePlasmaWindowManagement,
    SystemdUnits,
    AccessibilityWindows,
    UsageStats,
    /// A source of a newer host.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WindowList {
    #[serde(default)]
    pub windows: Vec<WindowEntry>,
    #[serde(default)]
    pub apps: Vec<AppEntry>,
    pub list_source: ListSource,
    /// Platform limits, e.g. that Wayland lets only the host user pick a
    /// window.
    #[serde(default)]
    pub limits: Vec<String>,
}

fn cut(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// Keeps the leading entries whose JSON (with a separating comma) fits into
/// `budget`, and takes their bytes from it.
fn fit<T: Serialize>(entries: &mut Vec<T>, budget: &mut usize) {
    let keep = entries
        .iter()
        .take_while(|entry| {
            let size = serde_json::to_vec(entry).map_or(usize::MAX, |json| json.len() + 1);
            let fits = size <= *budget;
            if fits {
                *budget -= size;
            }
            fits
        })
        .count();
    entries.truncate(keep);
}

impl WindowList {
    /// Cuts texts and lists so the `WINDOWS` frame stays within
    /// `MAX_WINDOWS_BYTES`: windows first, then apps, as many as fit once
    /// escaped as JSON, whatever their number. Entries with an id that is no
    /// valid source id are dropped.
    pub(crate) fn bounded(mut self) -> Self {
        self.windows.retain(|window| valid_id(&window.id));
        for window in &mut self.windows {
            cut(&mut window.title, MAX_TITLE_BYTES);
            cut(&mut window.app, MAX_NAME_BYTES);
            if let Some(monitor) = &mut window.monitor {
                cut(monitor, MAX_ID_BYTES);
            }
        }
        for app in &mut self.apps {
            cut(&mut app.app_id, MAX_NAME_BYTES);
            cut(&mut app.name, MAX_NAME_BYTES);
        }
        self.limits.truncate(MAX_LIMITS);
        for limit in &mut self.limits {
            cut(limit, MAX_NAME_BYTES);
        }
        // The rest of the frame (keys, list source, 16 limits of at most 64
        // bytes, escaped six-fold) stays below 16 KiB.
        let mut budget = MAX_WINDOWS_BYTES - 16 * 1024;
        fit(&mut self.windows, &mut budget);
        fit(&mut self.apps, &mut budget);
        self
    }

    /// The check of both sides beyond the frame size, which `read_frame`
    /// and `write_frame` already hold to `MAX_WINDOWS_BYTES`: any number of
    /// windows and apps, at most 16 limit notes.
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.limits.len() > MAX_LIMITS {
            return Err(invalid("screen window list has more than 16 limit notes"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "window_list_tests.rs"]
mod tests;
