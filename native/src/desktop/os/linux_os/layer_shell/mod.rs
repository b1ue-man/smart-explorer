//! The layer backend of se Desktop on Wayland (DSK-M7): `--desktop-surface
//! --layer` is one process for every output of the session. It draws the
//! surface view of the eframe windows (`show_surface`) with an egui host of
//! its own on `wlr-layer-shell` (layer `bottom`, namespace `desktop` under
//! KDE, else `smart-explorer-desktop`): one egui context, view model and
//! wgpu surface per output, one Wayland connection, one `calloop` loop and
//! one channel to the shell (standard input and output).
//!
//! `libwayland-client` and `libxkbcommon` are loaded at run time, so the
//! program links no Wayland library at build time (R78). There is no
//! software renderer: without graphics acceleration the process says so on
//! stderr and ends with exit code 1.
//!
//! Not part of it (`docs/oberflaechen/desktop-launcher.md`): dropping files
//! from other programs on the surface, dragging files out, Explorer actions
//! and input methods.
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::app::desktop_surface_app::LayerWake;
use crate::app::desktop_surface_view_model::ALIVE_EVERY;
use crate::daemon::UpdateStopWatch;
use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage};
use crate::desktop::os::feed_sources::DesktopFolder;
use crate::desktop::os::store::DesktopDir;
use crate::desktop::os::surface_link::FrameSender;

mod clipboard;
mod clipboard_uri;
mod event_loop;
mod host;
mod input;
mod keyboard;
mod keysyms;
mod pointer;
pub(crate) mod registry;
mod render;
mod startup;
mod state;
mod surface;
mod view;
mod xkb;

#[cfg(test)]
#[path = "layer_clipboard_tests.rs"]
mod layer_clipboard_tests;
#[cfg(test)]
#[path = "layer_task_env.rs"]
mod layer_task_env;
#[cfg(test)]
#[path = "layer_task_tests.rs"]
mod layer_task_tests;
#[cfg(test)]
#[path = "layer_tests.rs"]
mod layer_tests;

/// What `desktop_surface_app::run` hands over after the greeting.
pub(crate) struct LayerArgs {
    pub(crate) sender: FrameSender<SurfaceMessage>,
    pub(crate) inbox: Receiver<ShellMessage>,
    /// The suite switch `SE_DESKTOP_TEST_NO_ALIVE` turns the heartbeat off.
    pub(crate) send_alive: bool,
    /// The stored configuration until the shell sends its own.
    pub(crate) config: DesktopConfig,
    pub(crate) folders: Vec<DesktopFolder>,
    /// The loop installs its wake-up here for the channel's reader thread.
    pub(crate) wake: LayerWake,
    /// Kept for as long as the process runs: the update stop (75).
    pub(crate) watch: UpdateStopWatch,
}

/// States the reason on stderr and in the shell's log; stderr of a surface
/// is not connected to anything the user sees.
fn report(line: &str) {
    eprintln!("se Desktop: {line}");
    DesktopDir::production().log("layer_backend", line);
}

/// `--desktop-surface --layer`: the exit code of the process.
pub(crate) fn run_layer(args: LayerArgs) -> i32 {
    let LayerArgs {
        sender,
        inbox,
        send_alive,
        config,
        folders,
        wake,
        watch: _watch,
    } = args;
    let (ping, ping_source) = match calloop::ping::make_ping() {
        Ok(pair) => pair,
        Err(error) => {
            report(&format!("Layer-Backend: Weckruf: {error}"));
            return 1;
        }
    };
    if let Ok(mut slot) = wake.lock() {
        let wake_ping = ping.clone();
        *slot = Some(Box::new(move || wake_ping.ping()));
    }
    // The graphics may take a while to start; the shell waits ten seconds.
    if send_alive {
        sender.send(SurfaceMessage::Alive);
    }
    let shared = state::Shared {
        sender: Arc::new(sender),
        inbox,
        send_alive,
        next_alive: Instant::now() + ALIVE_EVERY,
        config,
        folders,
        repaints: Arc::new(Mutex::new(Vec::new())),
        ping,
        monitors: Vec::new(),
    };
    match startup::start(shared) {
        Ok((state, queue)) => event_loop::run(state, queue, ping_source),
        Err(failure) => {
            report(&failure.line);
            failure.code
        }
    }
}
