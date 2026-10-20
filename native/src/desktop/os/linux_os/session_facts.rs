//! What the Linux session offers se Desktop (U34-2, C2.2 inputs): the display
//! server from the environment, on X11 the atoms of the window manager, on
//! Wayland the version of `zwlr_layer_shell_v1`, on GNOME the Shell version
//! and an active desktop-icons extension, on COSMIC a running
//! `cosmic-files-applet`. The collecting sits behind [`Probe`], so the table
//! of C2.2 is checked without a display.
//!
//! Every query over D-Bus, X11 or Wayland runs under the time limit of
//! `probe_timeout` (two seconds); running out of time counts as "not
//! reachable". The shell collects the facts at its start, on every monitor
//! change and, while the GNOME Shell version is unknown, every five seconds.
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

use super::gnome_dbus::{parse_shell_major, SessionBus, ShellBus, STATE_ENABLED};
use super::layer_shell::registry;
use super::probe_timeout::timed;
use super::x11_facts;
use crate::desktop::core::mechanism::{DisplayServer, NetSupported, SessionFacts};

/// Desktop-icons extensions of GNOME that lie in the surface's way (C2.5,
/// assumption A20), in the order they are looked for.
pub(crate) const DESKTOP_ICON_EXTENSIONS: [&str; 2] =
    ["ding@rastersoft.com", "gtk4-ding@smedius.gitlab.com"];
const LAYER_SHELL: &str = "zwlr_layer_shell_v1";
const COSMIC_FILES: &str = "cosmic-files-applet";

/// What the X11 server announces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct X11Facts {
    /// The names in `_NET_SUPPORTED`; empty when the property is missing.
    pub(crate) supported: Vec<String>,
    /// A window of type DESKTOP of another program exists.
    pub(crate) stacking_has_foreign_desktop: bool,
    /// A compositor owns `_NET_WM_CM_S<screen>`.
    pub(crate) compositor: bool,
}

/// The sources of the facts.
pub(crate) trait Probe {
    fn env(&self, name: &str) -> Option<String>;
    fn x11_root_atoms(&self) -> Option<X11Facts>;
    fn wayland_layer_shell_version(&self) -> Option<u32>;
    fn gnome_shell_version(&self) -> Option<String>;
    /// UUIDs of the extensions the running Shell has enabled.
    fn gnome_active_extensions(&self) -> Vec<String>;
    /// A process of this user whose `comm` is `comm`.
    fn process_running(&self, comm: &str) -> bool;
}

/// The real session.
pub(crate) struct SystemProbe;

struct Registry;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Registry {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
    }
}

fn layer_shell_version() -> Option<u32> {
    let connection = registry::connect().ok()?;
    let (globals, _queue) = registry_queue_init::<Registry>(&connection).ok()?;
    globals.contents().with_list(|list| {
        list.iter()
            .find(|global| global.interface == LAYER_SHELL)
            .map(|global| global.version)
    })
}

fn same_user_process(comm: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: `getuid` has no arguments and cannot fail.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let numeric = name
            .to_str()
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()));
        numeric
            && entry.metadata().is_ok_and(|meta| meta.uid() == uid)
            && std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|text| text.trim() == comm)
    })
}

impl Probe for SystemProbe {
    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn x11_root_atoms(&self) -> Option<X11Facts> {
        timed("x11-facts", x11_facts::read).flatten()
    }

    fn wayland_layer_shell_version(&self) -> Option<u32> {
        timed("wayland-facts", layer_shell_version).flatten()
    }

    fn gnome_shell_version(&self) -> Option<String> {
        SessionBus.shell_version()
    }

    fn gnome_active_extensions(&self) -> Vec<String> {
        SessionBus
            .list_extensions()
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, state)| *state == STATE_ENABLED)
            .map(|(uuid, _)| uuid)
            .collect()
    }

    fn process_running(&self, comm: &str) -> bool {
        let comm = comm.to_string();
        timed("process-scan", move || same_user_process(&comm)).unwrap_or(false)
    }
}

fn non_empty(probe: &dyn Probe, name: &str) -> bool {
    probe.env(name).is_some_and(|value| !value.is_empty())
}

/// The display server from the environment: `WAYLAND_DISPLAY` (also with an
/// X11 `DISPLAY` beside it, XWayland), else `DISPLAY`, else none.
pub(crate) fn display_server(probe: &dyn Probe) -> DisplayServer {
    if non_empty(probe, "WAYLAND_DISPLAY") {
        DisplayServer::Wayland
    } else if non_empty(probe, "DISPLAY") {
        DisplayServer::X11
    } else {
        DisplayServer::Headless
    }
}

/// `XDG_CURRENT_DESKTOP` split at `:`.
pub(crate) fn current_desktop(probe: &dyn Probe) -> Vec<String> {
    probe
        .env("XDG_CURRENT_DESKTOP")
        .map(|value| {
            value
                .split(':')
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn has_desktop(desktops: &[String], name: &str) -> bool {
    desktops
        .iter()
        .any(|entry| entry.trim().eq_ignore_ascii_case(name))
}

fn net_supported(x11: &X11Facts) -> Option<NetSupported> {
    if x11.supported.is_empty() {
        return None;
    }
    let has = |name: &str| x11.supported.iter().any(|known| known == name);
    Some(NetSupported {
        window_type_desktop: has("_NET_WM_WINDOW_TYPE_DESKTOP"),
        state_below: has("_NET_WM_STATE_BELOW"),
    })
}

/// The facts of the session behind `probe` (U34-2).
pub(crate) fn probe(probe: &dyn Probe) -> SessionFacts {
    let display = display_server(probe);
    let desktops = current_desktop(probe);
    let mut facts = SessionFacts {
        display,
        current_desktop: desktops.clone(),
        ..SessionFacts::default()
    };
    match display {
        DisplayServer::X11 => {
            if let Some(x11) = probe.x11_root_atoms() {
                facts.net_supported = net_supported(&x11);
                facts.foreign_desktop_window = x11.stacking_has_foreign_desktop;
                facts.alpha_supported = x11.compositor;
            }
        }
        DisplayServer::Wayland => {
            facts.layer_shell_version = probe.wayland_layer_shell_version();
            // The layer process decides by the capabilities of its wgpu
            // surface (U37-8).
            facts.alpha_supported = true;
        }
        DisplayServer::Windows | DisplayServer::Headless => {}
    }
    if display != DisplayServer::Headless && has_desktop(&desktops, "GNOME") {
        facts.gnome_shell_major = probe
            .gnome_shell_version()
            .and_then(|text| parse_shell_major(&text));
        let active = probe.gnome_active_extensions();
        facts.foreign_desktop_icons = DESKTOP_ICON_EXTENSIONS
            .iter()
            .find(|uuid| active.iter().any(|known| known == *uuid))
            .map(|uuid| (*uuid).to_string());
    }
    if has_desktop(&desktops, "COSMIC") {
        facts.cosmic_files_desktop = probe.process_running(COSMIC_FILES);
    }
    facts
}

#[cfg(test)]
#[path = "session_facts_tests.rs"]
mod tests;
