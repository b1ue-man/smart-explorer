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
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage, PROTOCOL_VERSION};
use crate::desktop::os::store::DesktopDir;
use crate::desktop::os::surface_link::{self, FrameSender};
use crate::desktop::os::{feed_sources, platform, Session};

/// Lets the greeting leave before a test exit.
const TEST_EXIT_DELAY: Duration = Duration::from_millis(200);
/// App ID and X11 class of the surfaces (DSK-M6).
const APP_ID: &str = "app.smartexplorer.desktop";
/// Size of the window of the GUI harness.
const WINDOWED_SIZE: [f32; 2] = [1024.0, 768.0];

fn switch(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

fn monitor_name(target: &SurfaceTarget) -> String {
    match target {
        SurfaceTarget::Monitor { key } => key.clone(),
        SurfaceTarget::GnomeMonitor { index } => format!("gnome-{index}"),
        SurfaceTarget::Layer => "layer".into(),
    }
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
    let wake = move || {
        if let Ok(slot) = wake_slot.lock() {
            if let Some(ctx) = slot.as_ref() {
                ctx.request_repaint();
            }
        }
    };
    let (sender, inbox) = match surface_link::surface_stdio(wake) {
        Ok(channel) => channel,
        Err(error) => {
            eprintln!("se Desktop: Kanal: {error}");
            return 1;
        }
    };
    let monitor = monitor_name(target);
    sender.send(SurfaceMessage::Hello {
        protocol: PROTOCOL_VERSION,
        monitor: monitor.clone(),
        pid: std::process::id(),
        token: None,
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
    let windowed = switch("SE_DESKTOP_TEST_WINDOWED");
    // The stored configuration until the shell sends its own; it also says
    // whether the window needs an alpha channel.
    let stored = DesktopDir::production().read_config().ok().flatten();
    let transparent = !windowed && Session::probe(stored.as_ref()).transparent.is_ok();
    let config = stored.unwrap_or_else(|| DesktopConfig::initial(platform::initial_system_items()));
    let view = ViewModel::new(
        &monitor,
        Box::new(sender),
        send_alive,
        config,
        feed_sources::desktop_folders(),
    );
    let options = eframe::NativeOptions {
        viewport: viewport_for(target, windowed, transparent),
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
/// creation. The GUI harness shows an ordinary window instead.
fn viewport_for(
    target: &SurfaceTarget,
    windowed: bool,
    transparent: bool,
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
