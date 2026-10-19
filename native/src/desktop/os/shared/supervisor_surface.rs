//! The shell's surfaces (C5.5): one process per monitor with its channel,
//! the stop with five seconds of grace before the forced end, the halt after
//! a crash loop and the model a surface draws. The shell ends its own child
//! processes through their handles, never through a bare PID.
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use super::platform;
use super::store::{unix_now, DesktopDir};
use super::supervisor::TEST_MONITORS;
use super::surface_link::{self, FrameSender, LinkEvent};
use super::Monitor;
use crate::daemon::{UpdateStopBaseline, EXIT_UPDATE_STOP};
use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::items::{DesktopFile, SystemItem};
use crate::desktop::core::layout::DesktopLayout;
use crate::desktop::core::mechanism::Mechanism;
use crate::desktop::core::protocol::{
    ShellMessage, StopReason, SurfaceMessage, SurfaceModel, PROTOCOL_VERSION,
};
use crate::desktop::core::state::{Platform, State};
use crate::desktop::core::tiles::TileModel;

/// A stopped surface gets this long before it is ended by force (C5.5).
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(5);
const SURFACE_SWITCH: &str = "--desktop-surface";
const MONITOR_SWITCH: &str = "--monitor";
const LAYER_SWITCH: &str = "--layer";
/// The key of the one layer process (DSK-M7): it serves every output of the
/// session, so the shell counts it as one surface (`Surface::monitor`).
pub(crate) const LAYER_KEY: &str = "layer";

/// One running surface process.
pub(crate) struct Surface {
    pub(crate) id: u64,
    pub(crate) monitor: String,
    pub(crate) pid: u32,
    child: Child,
    link: FrameSender<ShellMessage>,
    pub(crate) last_alive: Instant,
    pub(crate) greeted: bool,
    pub(crate) sent_rev: Option<u64>,
    /// Since when the shell waits for a requested end.
    pub(crate) stopping: Option<Instant>,
    pub(crate) killed: bool,
}

impl Surface {
    pub(crate) fn send(&self, message: ShellMessage) -> bool {
        self.link.send(message)
    }

    pub(crate) fn stop(&mut self, reason: StopReason) {
        self.send(ShellMessage::Stop { reason });
        self.stopping = Some(Instant::now());
    }

    pub(crate) fn stop_overdue(&self, now: Instant) -> bool {
        self.stopping
            .is_some_and(|since| now.saturating_duration_since(since) >= STOP_GRACE)
    }

    pub(crate) fn kill(&mut self) {
        self.killed = true;
        let _ = self.child.kill();
    }

    /// `Some(code)` once the process ended; the inner `None` is an end by a
    /// signal.
    pub(crate) fn exited(&mut self) -> Option<Option<i32>> {
        self.child
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.code())
    }
}

/// Starts surface processes of the GUI program.
pub(crate) struct SurfaceSpawn {
    gui: PathBuf,
    events: Sender<(u64, LinkEvent<SurfaceMessage>)>,
    next_id: u64,
}

impl SurfaceSpawn {
    pub(crate) fn new(gui: PathBuf, events: Sender<(u64, LinkEvent<SurfaceMessage>)>) -> Self {
        SurfaceSpawn {
            gui,
            events,
            next_id: 1,
        }
    }

    pub(crate) fn gui(&self) -> &Path {
        &self.gui
    }

    /// `"<GUI>" --desktop-surface --monitor <key>` with its channel.
    pub(crate) fn start(&mut self, monitor: &str) -> Result<Surface, String> {
        let args = [SURFACE_SWITCH, MONITOR_SWITCH, monitor].map(str::to_string);
        self.start_with(monitor, &args)
    }

    /// `"<GUI>" --desktop-surface --layer` with its channel: one process for
    /// every output of the session (DSK-M7); its greeting names `layer`.
    pub(crate) fn start_layer(&mut self) -> Result<Surface, String> {
        let args = [SURFACE_SWITCH, LAYER_SWITCH].map(str::to_string);
        self.start_with(LAYER_KEY, &args)
    }

    fn start_with(&mut self, monitor: &str, args: &[String]) -> Result<Surface, String> {
        let mut child = platform::process::spawn_piped(&self.gui, args)
            .map_err(|error| format!("{}: {error}", self.gui.display()))?;
        let id = self.next_id;
        self.next_id += 1;
        let link = match surface_link::shell_side(&mut child, id, self.events.clone()) {
            Ok(link) => link,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        };
        link.send(hello_for(monitor));
        Ok(Surface {
            id,
            monitor: monitor.to_string(),
            pid: child.id(),
            child,
            link,
            last_alive: Instant::now(),
            greeted: false,
            sent_rev: None,
            stopping: None,
            killed: false,
        })
    }
}

/// Surfaces that get a process now: one per monitor (Windows, X11, the
/// suite's monitor keys), a single layer process for the whole session under
/// the layer backend, none under the GNOME extension (which starts its own).
pub(crate) fn wanted_surfaces(
    mechanism: &Mechanism,
    surface_on: bool,
    monitors: &[Monitor],
    test_monitors: Option<&[String]>,
) -> Vec<String> {
    if !surface_on {
        return Vec::new();
    }
    if let Some(keys) = test_monitors {
        return keys.to_vec();
    }
    match mechanism {
        Mechanism::WindowsBottom | Mechanism::X11DesktopType | Mechanism::X11Below => {
            monitors.iter().map(|monitor| monitor.key.clone()).collect()
        }
        Mechanism::LayerShell { .. } => vec![LAYER_KEY.to_string()],
        Mechanism::GnomeExtension { .. } | Mechanism::Unavailable(_) => Vec::new(),
    }
}

/// Sends the model to every greeted surface that has not seen revision
/// `rev`. A layer process gets one message per output, each with that
/// output's key and arrangement (DSK-M7); without outputs it gets nothing. A
/// surface whose sending fails keeps its old `sent_rev` and is served again.
pub(crate) fn send_models(
    surfaces: &mut [Surface],
    monitors: &[Monitor],
    rev: u64,
    feed: (&TileModel, &[DesktopFile], &DesktopConfig),
    layout: &mut DesktopLayout,
) {
    let (tiles, files, config) = feed;
    for surface in surfaces.iter_mut().filter(|surface| surface.greeted) {
        if surface.sent_rev == Some(rev) {
            continue;
        }
        if surface.monitor == LAYER_KEY {
            if monitors.is_empty() {
                continue;
            }
            let mut all_sent = true;
            for monitor in monitors {
                let Some(model) = surface_model(tiles, files, config, layout, &monitor.key) else {
                    continue;
                };
                all_sent &= surface.send(ShellMessage::Model {
                    rev,
                    monitor: Some(monitor.key.clone()),
                    model: Box::new(model),
                });
            }
            if all_sent {
                surface.sent_rev = Some(rev);
            }
            continue;
        }
        let Some(model) = surface_model(tiles, files, config, layout, &surface.monitor) else {
            continue;
        };
        if surface.send(ShellMessage::Model {
            rev,
            monitor: None,
            model: Box::new(model),
        }) {
            surface.sent_rev = Some(rev);
        }
    }
}

/// Sends `stop` to every surface, waits up to five seconds for their end and
/// ends the rest by force.
pub(crate) fn stop_all(dir: &DesktopDir, surfaces: &mut Vec<Surface>, reason: StopReason) {
    for surface in surfaces
        .iter_mut()
        .filter(|surface| surface.stopping.is_none())
    {
        surface.stop(reason);
    }
    let deadline = Instant::now() + STOP_GRACE;
    loop {
        surfaces.retain_mut(|surface| match surface.exited() {
            Some(code) => {
                let shown = code.map_or_else(|| "signal".to_string(), |code| code.to_string());
                dir.log(
                    "surface_exit",
                    &format!("{} {} {shown}", surface.monitor, surface.pid),
                );
                false
            }
            None => true,
        });
        if surfaces.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for surface in surfaces.iter_mut() {
        dir.log(
            "surface_killed",
            &format!("{} {}", surface.monitor, surface.pid),
        );
        surface.kill();
        let _ = surface.child.wait();
    }
    surfaces.clear();
}

/// What the halt needs of the shell.
pub(crate) struct HaltParts<'a> {
    pub(crate) dir: &'a DesktopDir,
    pub(crate) surfaces: &'a mut Vec<Surface>,
    pub(crate) update: &'a UpdateStopBaseline,
    pub(crate) restart: &'a UpdateStopBaseline,
}

/// Three crashes within ten minutes (C5.5): stop every surface, write
/// `halted` with this version, show the desktop notification and end. The
/// shell stays while the notification is shown and still ends at once on an
/// update stop, a restart request or a changed state.
pub(crate) fn halt(parts: HaltParts<'_>) -> i32 {
    let version = env!("CARGO_PKG_VERSION");
    stop_all(parts.dir, parts.surfaces, StopReason::Disable);
    write_halt(parts.dir, version);
    parts.dir.log("halt", version);
    crate::notify_desktop::notify(&crate::syncjobs::ProblemNotice {
        job_id: "se-desktop".into(),
        job_name: "se Desktop".into(),
        kind: crate::syncjobs::ProblemKind::NeedsAction,
        title: "se Desktop angehalten".into(),
        text: "se Desktop wurde nach wiederholten Abstürzen angehalten. Einstellungen → \
               Integration → se Desktop → „Neu starten“."
            .into(),
    });
    let deadline = Instant::now() + platform::NOTICE_LINGER;
    let mut code = 0;
    while Instant::now() < deadline {
        if parts.update.requested() {
            code = EXIT_UPDATE_STOP;
            break;
        }
        let enabled =
            matches!(parts.dir.read_state(), Ok(Some(state)) if state.state == State::Enabled);
        if parts.restart.requested() || !enabled {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    parts.dir.log("exit", &format!("{code} halted"));
    parts.dir.clear_shell();
    code
}

fn write_halt(dir: &DesktopDir, version: &str) {
    let Ok(_lock) = dir.lock() else {
        return;
    };
    if let Ok(Some(mut state)) = dir.read_state() {
        if state.state == State::Enabled {
            state.halt(version, unix_now());
            if let Err(error) = dir.write_state(&state) {
                dir.log("halt_unwritable", &error);
            }
        }
    }
}

/// The model of the surface on `monitor`; `None` for a key the arrangement
/// does not accept.
pub(crate) fn surface_model(
    tiles: &TileModel,
    files: &[DesktopFile],
    config: &DesktopConfig,
    layout: &mut DesktopLayout,
    monitor: &str,
) -> Option<SurfaceModel> {
    let system_items = if platform::PLATFORM == Platform::Windows {
        [
            (SystemItem::RecycleBin, config.system_items.recycle_bin),
            (SystemItem::ThisPc, config.system_items.this_pc),
        ]
        .into_iter()
        .filter(|(_, shown)| *shown)
        .map(|(item, _)| item)
        .collect()
    } else {
        Vec::new()
    };
    let layout = layout.monitor_mut(monitor).ok()?.clone();
    Some(SurfaceModel {
        tiles: tiles.clone(),
        files: files.to_vec(),
        system_items,
        screens: config.groups.screens,
        layout,
    })
}

/// The hello the shell sends first on a new channel.
pub(crate) fn hello_for(monitor: &str) -> ShellMessage {
    ShellMessage::Hello {
        protocol: PROTOCOL_VERSION,
        monitor: monitor.to_string(),
    }
}

/// The suite switch `SE_DESKTOP_TEST_MONITORS` (DSK-M15): monitor keys
/// instead of the session's monitors; never set in production.
pub(crate) fn test_monitors() -> Option<Vec<String>> {
    let value = std::env::var(TEST_MONITORS).ok()?;
    let keys: Vec<String> = value
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .collect();
    (!keys.is_empty()).then_some(keys)
}

#[cfg(all(test, unix))]
#[path = "supervisor_surface_tests.rs"]
mod tests;
