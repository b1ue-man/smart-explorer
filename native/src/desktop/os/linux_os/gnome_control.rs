//! The control files between the shell and the GNOME extension (U34-12), all
//! in `$XDG_RUNTIME_DIR/smart-explorer/` (folder 0700, files 0600), written
//! atomically (helper file and rename):
//!
//! - `desktop-gnome.json`, written by the shell and deleted at its end: the
//!   protocol (1), the GUI program, a generation that grows by one with every
//!   change of the monitor list, the socket, and the monitors (index, key and
//!   the rectangle Mutter reports, for the extension's check of its monitor
//!   order);
//! - `desktop.token`: 128 random bits as 32 hex digits, new at every start
//!   of the shell;
//! - `desktop.sock`: the socket the surfaces connect to (`gnome_channel`);
//! - `desktop-extension.json`, written by the extension: its variant, the
//!   surfaces it started and whether it placed their windows, and a halt
//!   after a crash loop. The shell only reads it, for the status.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::session_facts::{self, Probe, SystemProbe, X11Facts};
use crate::desktop::core::mechanism::SessionFacts;

pub(crate) const CONTROL_FILE: &str = "desktop-gnome.json";
pub(crate) const TOKEN_FILE: &str = "desktop.token";
pub(crate) const SOCKET_FILE: &str = "desktop.sock";
pub(crate) const EXTENSION_FILE: &str = "desktop-extension.json";
/// The protocol number of both control files.
pub(crate) const PROTOCOL: u32 = 1;
/// A control file is a few hundred bytes; anything larger is not ours.
const MAX_FILE: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ControlMonitor {
    pub(crate) index: u32,
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) x: i32,
    #[serde(default)]
    pub(crate) y: i32,
    #[serde(default)]
    pub(crate) width: i32,
    #[serde(default)]
    pub(crate) height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Control {
    pub(crate) protocol: u32,
    pub(crate) exe: String,
    pub(crate) generation: u64,
    pub(crate) socket: String,
    pub(crate) monitors: Vec<ControlMonitor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExtensionSurface {
    pub(crate) monitor: u32,
    pub(crate) pid: u32,
    pub(crate) handled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Halted {
    pub(crate) reason: String,
    pub(crate) at: i64,
}

/// `desktop-extension.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExtensionReport {
    pub(crate) version: u32,
    pub(crate) variant: String,
    pub(crate) surfaces: Vec<ExtensionSurface>,
    #[serde(default)]
    pub(crate) halted: Option<Halted>,
}

impl ExtensionReport {
    /// At least one surface whose window the extension placed.
    pub(crate) fn handles_a_window(&self) -> bool {
        self.surfaces.iter().any(|surface| surface.handled)
    }
}

/// The runtime folder of this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeFiles {
    dir: PathBuf,
}

impl RuntimeFiles {
    pub(crate) fn at(dir: PathBuf) -> RuntimeFiles {
        RuntimeFiles { dir }
    }

    /// `$XDG_RUNTIME_DIR/smart-explorer`; `None` without a runtime folder.
    pub(crate) fn from_env() -> Option<RuntimeFiles> {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .map(|dir| RuntimeFiles::at(dir.join("smart-explorer")))
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn control_path(&self) -> PathBuf {
        self.dir.join(CONTROL_FILE)
    }

    pub(crate) fn token_path(&self) -> PathBuf {
        self.dir.join(TOKEN_FILE)
    }

    pub(crate) fn socket_path(&self) -> PathBuf {
        self.dir.join(SOCKET_FILE)
    }

    pub(crate) fn extension_path(&self) -> PathBuf {
        self.dir.join(EXTENSION_FILE)
    }

    fn write(&self, path: &Path, text: &str) -> Result<(), String> {
        crate::support_dirs::write_private_atomic(path, text.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn read(path: &Path) -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > MAX_FILE {
            return None;
        }
        std::fs::read_to_string(path).ok()
    }

    pub(crate) fn write_control(&self, control: &Control) -> Result<(), String> {
        let text = serde_json::to_string(control).map_err(|error| error.to_string())?;
        self.write(&self.control_path(), &text)
    }

    pub(crate) fn read_control(&self) -> Option<Control> {
        serde_json::from_str(&Self::read(&self.control_path())?).ok()
    }

    /// A new token, written for the surfaces and the listener.
    pub(crate) fn new_token(&self) -> Result<String, String> {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
        let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        self.write(&self.token_path(), &token)?;
        Ok(token)
    }

    pub(crate) fn read_token(&self) -> Option<String> {
        let token = Self::read(&self.token_path())?.trim().to_string();
        (token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(token)
    }

    pub(crate) fn read_extension(&self) -> Option<ExtensionReport> {
        serde_json::from_str(&Self::read(&self.extension_path())?).ok()
    }

    /// What a surface needs to connect: the socket and the token.
    pub(crate) fn surface_side(&self) -> Result<(PathBuf, String), String> {
        let control = self
            .read_control()
            .filter(|control| control.protocol == PROTOCOL)
            .ok_or("desktop-gnome.json fehlt oder ist unlesbar")?;
        let token = self.read_token().ok_or("desktop.token fehlt")?;
        Ok((PathBuf::from(control.socket), token))
    }

    pub(crate) fn remove_control(&self) {
        let _ = std::fs::remove_file(self.control_path());
    }

    /// The shell's end: its control file, token and socket.
    pub(crate) fn remove_shell_files(&self) {
        for path in [self.control_path(), self.token_path(), self.socket_path()] {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// What the session probe of a surface the extension started may ask. The
/// extension's `Meta.WaylandClient` gives the surface one Wayland connection,
/// the socket named by `WAYLAND_SOCKET`, and the Shell counts a window as the
/// extension's only when it arrives over that connection. The first
/// connection a process makes from the environment takes the socket, so the
/// probe of `layer_shell_version` (a connection, two round trips, closed
/// again) left the window system's own connection to `WAYLAND_DISPLAY`, a
/// client the extension does not own: the window was never placed and
/// `handled` stayed false (suite run 6,
/// `desktop_task_gnome_headless_variant_a`). The extension starts surfaces
/// only where the session has no layer shell (C2.2 decides the layer shell
/// first), so this probe does not ask for it and touches no Wayland
/// connection; everything else is read as everywhere.
struct SurfaceProbe;

impl Probe for SurfaceProbe {
    fn env(&self, name: &str) -> Option<String> {
        SystemProbe.env(name)
    }

    fn x11_root_atoms(&self) -> Option<X11Facts> {
        SystemProbe.x11_root_atoms()
    }

    fn wayland_layer_shell_version(&self) -> Option<u32> {
        None
    }

    fn gnome_shell_version(&self) -> Option<String> {
        SystemProbe.gnome_shell_version()
    }

    fn gnome_active_extensions(&self) -> Vec<String> {
        SystemProbe.gnome_active_extensions()
    }

    fn process_running(&self, comm: &str) -> bool {
        SystemProbe.process_running(comm)
    }
}

/// The facts of the session for a surface the extension started.
pub(crate) fn surface_session_facts() -> SessionFacts {
    session_facts::probe(&SurfaceProbe)
}
