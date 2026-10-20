//! The surface process `--desktop-surface` (DSK-M3 frame, DSK-M4 view): one
//! window per monitor without decorations, task-bar entry or minimize
//! button, kept at the bottom (C1.2; DSK-M5 and DSK-M6 pin it there and fit
//! the monitor's physical rectangle). It takes the shell's model and
//! configuration from its channel, draws the surface (`show_surface`), asks
//! for a frame at least every second and sends `alive` at most once a second
//! from the UI thread (C5.5). The end of the channel ends it with 76, an
//! update stop with 75 (V8), whichever comes first. The window is
//! transparent only where the session gives it an alpha channel (C5.4).
//!
//! Suite switches (DSK-M15), never set in production:
//! `SE_DESKTOP_TEST_HEADLESS=1` runs channel, update stop and heartbeat
//! without a window; `SE_DESKTOP_TEST_WINDOWED=1` shows the surface as an
//! ordinary window (GUI harness); `SE_DESKTOP_TEST_NO_ALIVE=1` sends no
//! heartbeat; `SE_DESKTOP_TEST_SURFACE_EXIT=<code>` ends with that code right
//! after the greeting; `SE_DESKTOP_TEST_CONNECT=1` presses "Verbinden" once.
//!
//! `--layer` (DSK-M7, Wayland) is one process for every output of the
//! session with its own egui host (`desktop::os::linux_os::layer_shell`);
//! the greeting, the test switches, the heartbeat, the update stop and the
//! channel are those of this function, only the window and the loop differ.
//!
//! `--gnome-monitor <index>` (DSK-M8) is started by the GNOME Shell
//! extension, not by the shell: it connects to the socket of
//! `desktop-gnome.json`, greets with the token of `desktop.token` as monitor
//! `gnome-<index>` and then works under the key that file names for the
//! index. The extension places its window; the view is the eframe one.
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

use super::desktop_surface_actions::SystemActions;
use super::desktop_surface_grid_ui::{show_surface, SurfaceState};
use super::desktop_surface_host::{EframeHost, SurfaceHost};
use super::desktop_surface_view_model::{ViewModel, ALIVE_EVERY};
use crate::app::start_request::SurfaceTarget;
use crate::daemon::{spawn_update_stop_watch, UpdateStopWatch, EXIT_UPDATE_STOP, UPDATE_STOP_POLL};
use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::mechanism::Mechanism;
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage, PROTOCOL_VERSION};
use crate::desktop::os::store::DesktopDir;
use crate::desktop::os::surface_link::{self, FrameSender};
use crate::desktop::os::{feed_sources, platform, Session};

/// Wakes the layer backend's event loop when the channel delivered a message
/// (the loop installs its ping once it exists).
pub(crate) type LayerWake = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

/// Lets the greeting leave before a test exit.
const TEST_EXIT_DELAY: Duration = Duration::from_millis(200);
/// App ID and X11 class of the surfaces (DSK-M6).
const APP_ID: &str = "app.smartexplorer.desktop";
/// Size of the window of the GUI harness.
const WINDOWED_SIZE: [f32; 2] = [1024.0, 768.0];

fn switch(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

/// The name in the greeting.
fn monitor_name(target: &SurfaceTarget) -> String {
    match target {
        SurfaceTarget::Monitor { key } => key.clone(),
        SurfaceTarget::GnomeMonitor { index } => format!("gnome-{index}"),
        SurfaceTarget::Layer => "layer".into(),
    }
}

/// What a surface connects with: its channel, the token of a GNOME socket
/// and the key of its monitor (`layout/<key>.json`).
struct Link {
    sender: FrameSender<SurfaceMessage>,
    inbox: Receiver<ShellMessage>,
    token: Option<String>,
    key: String,
}

/// The surface of the GNOME extension: the socket and the token of the
/// shell, the monitor key of its index.
#[cfg(target_os = "linux")]
fn gnome_link(index: u32, wake: impl Fn() + Send + 'static) -> Result<Link, String> {
    use crate::desktop::os::platform::gnome_channel::surface_socket;
    use crate::desktop::os::platform::gnome_control::RuntimeFiles;
    let files = RuntimeFiles::from_env().ok_or("XDG_RUNTIME_DIR fehlt")?;
    let (socket, token) = files.surface_side()?;
    let key = files
        .read_control()
        .and_then(|control| {
            control
                .monitors
                .into_iter()
                .find(|monitor| monitor.index == index)
        })
        .map(|monitor| monitor.key)
        .ok_or_else(|| format!("Monitor {index} steht nicht in desktop-gnome.json"))?;
    let (sender, inbox) =
        surface_socket(&socket, &token, wake).map_err(|error| format!("Wächter: {error}"))?;
    Ok(Link {
        sender,
        inbox,
        token: Some(token),
        key,
    })
}

#[cfg(not(target_os = "linux"))]
fn gnome_link(_index: u32, _wake: impl Fn() + Send + 'static) -> Result<Link, String> {
    Err("die GNOME-Erweiterung gibt es nur unter Linux".into())
}

fn stdio_link(target: &SurfaceTarget, wake: impl Fn() + Send + 'static) -> Result<Link, String> {
    let (sender, inbox) = surface_link::surface_stdio(wake).map_err(|error| error.to_string())?;
    Ok(Link {
        sender,
        inbox,
        token: None,
        key: monitor_name(target),
    })
}

/// `--desktop-surface …`: the exit code of the process.
pub(crate) fn run(target: &SurfaceTarget) -> i32 {
    // V8 first: a new update stop ends the surface with 75.
    let watch = match spawn_update_stop_watch(UPDATE_STOP_POLL, || {
        std::process::exit(EXIT_UPDATE_STOP);
    }) {
        Ok(watch) => watch,
        Err(error) => {
            eprintln!("se Desktop: Update-Stopp-Wache: {error}");
            return 1;
        }
    };
    let waker: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let wake_slot = waker.clone();
    let layer_wake: LayerWake = Arc::new(Mutex::new(None));
    let layer_slot = layer_wake.clone();
    let wake = move || {
        if let Ok(slot) = wake_slot.lock() {
            if let Some(ctx) = slot.as_ref() {
                ctx.request_repaint();
            }
        }
        if let Ok(slot) = layer_slot.lock() {
            if let Some(ping) = slot.as_ref() {
                ping();
            }
        }
    };
    let linked = match target {
        SurfaceTarget::GnomeMonitor { index } => gnome_link(*index, wake),
        _ => stdio_link(target, wake),
    };
    let Link {
        sender,
        inbox,
        token,
        key: monitor,
    } = match linked {
        Ok(link) => link,
        Err(error) => {
            eprintln!("se Desktop: Kanal: {error}");
            return 1;
        }
    };
    sender.send(SurfaceMessage::Hello {
        protocol: PROTOCOL_VERSION,
        monitor: monitor_name(target),
        pid: std::process::id(),
        token,
    });
    if let Some(code) = std::env::var("SE_DESKTOP_TEST_SURFACE_EXIT")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
    {
        std::thread::sleep(TEST_EXIT_DELAY);
        std::process::exit(code);
    }
    if switch("SE_DESKTOP_TEST_CONNECT") {
        sender.send(SurfaceMessage::Connect);
    }
    let send_alive = !switch("SE_DESKTOP_TEST_NO_ALIVE");
    if switch("SE_DESKTOP_TEST_HEADLESS") {
        run_headless(&sender, &inbox, send_alive, watch);
    }
    if matches!(target, SurfaceTarget::Layer) {
        return start_layer_backend(sender, inbox, send_alive, layer_wake, watch);
    }
    let windowed = switch("SE_DESKTOP_TEST_WINDOWED");
    // The stored configuration until the shell sends its own; it also says
    // whether the window needs an alpha channel.
    let stored = DesktopDir::production().read_config().ok().flatten();
    let session = (!windowed).then(|| Session::probe(stored.as_ref()));
    let transparent = session
        .as_ref()
        .is_some_and(|session| session.transparent.is_ok());
    // X11: the window type DESKTOP is set before the window is shown.
    let desktop_type = session
        .as_ref()
        .is_some_and(|session| session.mechanism == Mechanism::X11DesktopType);
    let config = stored.unwrap_or_else(|| DesktopConfig::initial(platform::initial_system_items()));
    let view = ViewModel::new(
        &monitor,
        Box::new(sender),
        send_alive,
        config,
        feed_sources::desktop_folders(),
    );
    let options = eframe::NativeOptions {
        viewport: viewport_for(target, windowed, transparent, desktop_type),
        ..Default::default()
    };
    let result = eframe::run_native(
        "se Desktop",
        options,
        Box::new(move |cc| {
            if let Ok(mut slot) = waker.lock() {
                *slot = Some(cc.egui_ctx.clone());
            }
            Ok(Box::new(SurfaceApp {
                view,
                inbox,
                host: EframeHost::new(cc, &monitor),
                state: SurfaceState::new(transparent),
                actions: SystemActions::new(cc.egui_ctx.clone()),
                transparent,
                _watch: watch,
            }))
        }),
    );
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("se Desktop: {error}");
            1
        }
    }
}

/// The layer backend: the stored configuration until the shell sends its
/// own, then the loop of `layer_shell` for every output; its exit code.
#[cfg(target_os = "linux")]
fn start_layer_backend(
    sender: FrameSender<SurfaceMessage>,
    inbox: Receiver<ShellMessage>,
    send_alive: bool,
    wake: LayerWake,
    watch: UpdateStopWatch,
) -> i32 {
    use crate::desktop::os::platform::layer_shell::{run_layer, LayerArgs};
    let stored = DesktopDir::production().read_config().ok().flatten();
    let config = stored.unwrap_or_else(|| DesktopConfig::initial(platform::initial_system_items()));
    run_layer(LayerArgs {
        sender,
        inbox,
        send_alive,
        config,
        folders: feed_sources::desktop_folders(),
        wake,
        watch,
    })
}

#[cfg(not(target_os = "linux"))]
fn start_layer_backend(
    _sender: FrameSender<SurfaceMessage>,
    _inbox: Receiver<ShellMessage>,
    _send_alive: bool,
    _wake: LayerWake,
    _watch: UpdateStopWatch,
) -> i32 {
    eprintln!("se Desktop: Layer-Backend: gibt es nur unter Linux (Wayland)");
    1
}

/// The suite's surface without a window: the reader thread ends the process
/// on `stop` and at the channel end; the watch on an update stop.
fn run_headless(
    sender: &FrameSender<SurfaceMessage>,
    inbox: &Receiver<ShellMessage>,
    send_alive: bool,
    _watch: UpdateStopWatch,
) -> ! {
    loop {
        while inbox.try_recv().is_ok() {}
        if send_alive {
            sender.send(SurfaceMessage::Alive);
        }
        std::thread::sleep(ALIVE_EVERY);
    }
}

/// C1.2 window shape. The position is the monitor's rectangle as the
/// platform reports it; DSK-M5 sets the exact physical rectangle after
/// creation. `desktop_type`: X11 with the window type DESKTOP (L1). The GUI
/// harness shows an ordinary window instead.
fn viewport_for(
    target: &SurfaceTarget,
    windowed: bool,
    transparent: bool,
    desktop_type: bool,
) -> egui::ViewportBuilder {
    if windowed {
        return egui::ViewportBuilder::default()
            .with_title(format!("se Desktop ({})", monitor_name(target)))
            .with_app_id(APP_ID)
            .with_inner_size(WINDOWED_SIZE);
    }
    let mut builder = egui::ViewportBuilder::default()
        .with_title("se Desktop")
        .with_app_id(APP_ID)
        .with_decorations(false)
        .with_taskbar(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_transparent(transparent)
        .with_window_level(egui::WindowLevel::AlwaysOnBottom);
    if desktop_type {
        builder = builder.with_window_type(egui::X11WindowType::Desktop);
    }
    if let SurfaceTarget::Monitor { key } = target {
        let rect = platform::monitors()
            .into_iter()
            .find(|monitor| &monitor.key == key)
            .and_then(|monitor| monitor.rect);
        if let Some([left, top, right, bottom]) = rect {
            builder = builder
                .with_position([left as f32, top as f32])
                .with_inner_size([(right - left) as f32, (bottom - top) as f32]);
        }
    }
    builder
}

struct SurfaceApp {
    view: ViewModel,
    inbox: Receiver<ShellMessage>,
    host: EframeHost,
    state: SurfaceState,
    actions: SystemActions,
    transparent: bool,
    _watch: UpdateStopWatch,
}

impl eframe::App for SurfaceApp {
    /// Transparent only with an alpha channel; the wallpaper covers the rest.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.transparent {
            [0.0; 4]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.view.drain(&self.inbox) {
            self.host.request_repaint();
        }
        show_surface(
            ctx,
            &mut self.view,
            &mut self.state,
            &self.host,
            &mut self.actions,
        );
        ctx.request_repaint_after(ALIVE_EVERY);
        self.view.heartbeat();
    }
}
