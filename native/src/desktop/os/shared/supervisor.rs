//! The se Desktop shell `--desktop-shell` (C5.5, C5.6, C9): a windowless
//! process of the GUI image, one per user. It starts one surface per monitor,
//! is their only data holder, watches their heartbeats and restarts them
//! (`crash_rule`), follows `state.json` (disabled or removed: end within a
//! second), `shell.stop` (restart) and the update stop marker (V8: stop the
//! surfaces, end with 75). It never starts the background service (V11).
//! Once a second it also looks at the monitors and keeps the platform's
//! other parts current (DSK-M5 Windows: navigation pane and jump list,
//! `supervisor_upkeep.rs`).
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::crash_rule::{CrashRule, CrashVerdict};
use super::layout_store::{self, LayoutStore};
use super::model_feed::{FeedSources, ModelFeed};
use super::store::{DesktopDir, ShellRuntime};
use super::supervisor_surface::{Surface, SurfaceSpawn};
use super::surface_link::LinkEvent;
use super::{platform, Monitor};
use crate::daemon::{UpdateStopBaseline, EXIT_UPDATE_STOP};
use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::layout::DesktopLayout;
use crate::desktop::core::mechanism::{self, Mechanism};
use crate::desktop::core::protocol::EXIT_CHANNEL_END;
use crate::desktop::core::protocol::{check_hello, ShellMessage, StopReason, SurfaceMessage};
use crate::desktop::core::state::{ModuleState, State};

#[path = "supervisor_upkeep.rs"]
mod upkeep;

/// A surface without `alive` for this long hangs (C5.5, Annahme A45).
pub(crate) const ALIVE_TIMEOUT: Duration = Duration::from_secs(10);
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
const LOOP_TICK: Duration = Duration::from_millis(100);
/// A status probe may hold the instance for an instant.
const INSTANCE_WAIT: Duration = Duration::from_secs(2);
/// Suite switch (DSK-M15): monitor keys instead of the session's monitors.
pub(crate) const TEST_MONITORS: &str = "SE_DESKTOP_TEST_MONITORS";

enum Watch {
    Continue,
    End(&'static str),
}

/// `--desktop-shell`: the exit code of the process.
pub(crate) fn run_shell() -> i32 {
    // V8: the baseline first, so only a later update request ends the shell.
    let update = UpdateStopBaseline::arm();
    let dir = DesktopDir::production();
    let version = env!("CARGO_PKG_VERSION");
    let Ok(Some(state)) = dir.read_state() else {
        return 0;
    };
    if !state.should_resume(version) {
        return 0;
    }
    let Ok(gui) = std::env::current_exe() else {
        return 1;
    };
    let instance = match platform::instance::acquire(&dir, INSTANCE_WAIT) {
        Ok(Some(instance)) => instance,
        Ok(None) => return 0,
        Err(error) => {
            dir.log("error", &error);
            return 1;
        }
    };
    let stop_dir = dir.clone();
    let restart = UpdateStopBaseline::with_source(Arc::new(move || stop_dir.stop_marker()));
    let feed = match ModelFeed::start(FeedSources::system(), dir.clone()) {
        Ok(feed) => feed,
        Err(error) => {
            dir.log("error", &error.to_string());
            return 1;
        }
    };
    let (events_tx, events) = mpsc::channel();
    let (layouts, layout) = LayoutStore::open(dir.clone());
    let mut shell = Shell {
        parts: platform::ShellParts::new(dir.clone(), gui.clone()),
        seen: None,
        spawn: SurfaceSpawn::new(gui, events_tx),
        dir,
        surfaces: Vec::new(),
        restart_at: Vec::new(),
        crashes: CrashRule::default(),
        events,
        feed,
        layout,
        layouts,
        config: None,
        monitors: Vec::new(),
        mechanism: Mechanism::Unavailable(mechanism::Reason::NoGraphicalSession),
        surface_on: state.parts.surface,
        model_rev: 0,
        feed_rev: 0,
        runtime: None,
        update,
        restart,
        test_monitors: super::supervisor_surface::test_monitors(),
    };
    let code = shell.run();
    drop(instance);
    code
}

struct Shell {
    dir: DesktopDir,
    spawn: SurfaceSpawn,
    surfaces: Vec<Surface>,
    /// Monitor → earliest restart after a crash.
    restart_at: Vec<(String, Instant)>,
    crashes: CrashRule,
    events: Receiver<(u64, LinkEvent<SurfaceMessage>)>,
    feed: ModelFeed,
    layout: DesktopLayout,
    /// The only writer of `layout/*.json` (DSK-M4).
    layouts: LayoutStore,
    config: Option<DesktopConfig>,
    monitors: Vec<Monitor>,
    /// Decided at the start and again on a monitor or configuration change.
    mechanism: Mechanism,
    surface_on: bool,
    /// `state.json` as read in the last round.
    seen: Option<ModuleState>,
    /// The platform's parts besides the surfaces (DSK-M5).
    parts: platform::ShellParts,
    model_rev: u64,
    feed_rev: u64,
    runtime: Option<ShellRuntime>,
    update: UpdateStopBaseline,
    restart: UpdateStopBaseline,
    test_monitors: Option<Vec<String>>,
}

impl Shell {
    fn run(&mut self) -> i32 {
        let version = env!("CARGO_PKG_VERSION");
        self.dir.log(
            "start",
            &format!("pid {} version {version}", std::process::id()),
        );
        self.reload_config();
        self.refresh_monitors();
        let mut last_watch = Instant::now();
        loop {
            match self.events.recv_timeout(LOOP_TICK) {
                Ok((id, event)) => self.on_event(id, event),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
            while let Ok((id, event)) = self.events.try_recv() {
                self.on_event(id, event);
            }
            if last_watch.elapsed() >= WATCH_INTERVAL {
                last_watch = Instant::now();
                if self.update.requested() {
                    return self.end_for_update();
                }
                if self.restart.requested() {
                    return self.end(StopReason::Restart, 0, "restart");
                }
                if let Watch::End(reason) = self.watch_state() {
                    return self.end(StopReason::Disable, 0, reason);
                }
                self.reload_config();
                self.upkeep();
            }
            if let Some(code) = self.reap() {
                return code;
            }
            if let Some(code) = self.ensure_surfaces() {
                return code;
            }
            self.push_models();
            if let Err(error) = self.layouts.save_at(&self.layout, self.model_rev) {
                self.dir.log("layout_unwritable", &error);
            }
            self.write_runtime();
        }
    }

    fn watch_state(&mut self) -> Watch {
        match self.dir.read_state() {
            Ok(Some(state)) if state.state == State::Enabled && state.parts.any() => {
                self.surface_on = state.parts.surface;
                self.seen = Some(state);
                Watch::Continue
            }
            Ok(Some(_)) => Watch::End("disabled"),
            Ok(None) => Watch::End("absent"),
            // A half-written or broken file must not end the shell.
            Err(error) => {
                self.dir.log("state_unreadable", &error);
                Watch::Continue
            }
        }
    }

    fn reload_config(&mut self) {
        let config = self.dir.read_config().ok().flatten();
        if config != self.config {
            self.config = config;
            self.model_rev += 1;
            self.mechanism = self.decide_mechanism();
            if let Some(config) = &self.config {
                for surface in self.surfaces.iter().filter(|surface| surface.greeted) {
                    surface.send(ShellMessage::Config {
                        config: config.clone(),
                    });
                }
            }
        }
    }

    fn refresh_monitors(&mut self) {
        self.monitors = platform::monitors();
        self.mechanism = self.decide_mechanism();
        self.model_rev += 1;
        // C5.5: the primary monitor keeps the items of a monitor that left.
        if let Some(primary) = self.primary_monitor() {
            let present = self.wanted_monitors();
            let _ = self.layout.adopt_removed_monitors(&primary, &present);
        }
    }

    fn decide_mechanism(&self) -> Mechanism {
        let icons = self.config.as_ref().map_or(
            crate::desktop::core::config::ForeignDesktopIcons::Keep,
            |config| config.foreign_desktop_icons,
        );
        mechanism::decide(&platform::session_facts(), icons)
    }

    /// Monitors that get a surface process now (`--monitor <key>`).
    fn wanted_monitors(&self) -> Vec<String> {
        if !self.surface_on {
            return Vec::new();
        }
        if let Some(keys) = &self.test_monitors {
            return keys.clone();
        }
        match &self.mechanism {
            // One process per monitor (Windows, X11).
            Mechanism::WindowsBottom | Mechanism::X11DesktopType | Mechanism::X11Below => self
                .monitors
                .iter()
                .map(|monitor| monitor.key.clone())
                .collect(),
            // The layer backend (DSK-M7) and the GNOME extension (DSK-M8)
            // start their surfaces themselves; nothing here is unavailable.
            Mechanism::LayerShell { .. }
            | Mechanism::GnomeExtension { .. }
            | Mechanism::Unavailable(_) => Vec::new(),
        }
    }

    fn primary_monitor(&self) -> Option<String> {
        let wanted = self.wanted_monitors();
        self.monitors
            .iter()
            .find(|monitor| monitor.primary && wanted.contains(&monitor.key))
            .map(|monitor| monitor.key.clone())
            .or_else(|| wanted.first().cloned())
    }

    /// Starts the missing surfaces; `Some(code)` when start failures halted
    /// the shell.
    fn ensure_surfaces(&mut self) -> Option<i32> {
        let wanted = self.wanted_monitors();
        for surface in &mut self.surfaces {
            if !wanted.contains(&surface.monitor) && surface.stopping.is_none() {
                surface.stop(StopReason::Disable);
            }
        }
        let now = Instant::now();
        self.restart_at.retain(|(_, at)| *at > now);
        for monitor in wanted {
            let running = self
                .surfaces
                .iter()
                .any(|surface| surface.monitor == monitor);
            let waiting = self.restart_at.iter().any(|(key, _)| *key == monitor);
            if running || waiting {
                continue;
            }
            match self.spawn.start(&monitor) {
                Ok(surface) => {
                    self.dir
                        .log("surface_start", &format!("{monitor} {}", surface.pid));
                    self.surfaces.push(surface);
                }
                Err(error) => {
                    self.dir
                        .log("surface_spawn_failed", &format!("{monitor} {error}"));
                    if let Some(code) = self.crashed(&monitor) {
                        return Some(code);
                    }
                }
            }
        }
        None
    }

    fn on_event(&mut self, id: u64, event: LinkEvent<SurfaceMessage>) {
        let Some(index) = self.surfaces.iter().position(|surface| surface.id == id) else {
            return;
        };
        let LinkEvent::Message(message) = event else {
            // The end is handled when the process is reaped.
            return;
        };
        match message {
            hello @ SurfaceMessage::Hello { .. } => {
                let surface = &mut self.surfaces[index];
                if let Err(error) = check_hello(&hello, Some(surface.monitor.as_str()), None) {
                    self.dir.log("surface_hello_rejected", &error.to_string());
                    surface.kill();
                    return;
                }
                surface.greeted = true;
                surface.last_alive = Instant::now();
                surface.sent_rev = None;
                if let Some(config) = &self.config {
                    surface.send(ShellMessage::Config {
                        config: config.clone(),
                    });
                }
            }
            SurfaceMessage::Alive => self.surfaces[index].last_alive = Instant::now(),
            SurfaceMessage::Layout { op } => {
                if self.layout.apply(&op).is_ok() {
                    self.model_rev += 1;
                }
            }
            SurfaceMessage::MonitorsChanged => self.poll_monitors(),
            // The wallpaper is read again by DSK-M4/DSK-M5.
            SurfaceMessage::WallpaperChanged => self.model_rev += 1,
            SurfaceMessage::Connect => self.feed.connect(),
        }
    }

    /// Ended surfaces and hanging ones; `Some(code)` when the shell ends.
    fn reap(&mut self) -> Option<i32> {
        let now = Instant::now();
        let mut index = 0;
        while index < self.surfaces.len() {
            let surface = &mut self.surfaces[index];
            let Some(code) = surface.exited() else {
                let silent = now.saturating_duration_since(surface.last_alive);
                if surface.killed {
                    // Reaped in a following round.
                } else if surface.stopping.is_none() && silent >= ALIVE_TIMEOUT {
                    self.dir.log(
                        "surface_hung",
                        &format!("{} {}", surface.monitor, surface.pid),
                    );
                    surface.kill();
                } else if surface.stop_overdue(now) {
                    surface.kill();
                }
                index += 1;
                continue;
            };
            let surface = self.surfaces.remove(index);
            let shown = code.map_or_else(|| "signal".to_string(), |code| code.to_string());
            self.dir.log(
                "surface_exit",
                &format!("{} {} {shown}", surface.monitor, surface.pid),
            );
            self.model_rev += 1;
            if surface.stopping.is_some() {
                continue;
            }
            match code {
                // Not crashes (C5.5): restart without counting; on an update
                // the shell sees the marker within a second and ends first.
                Some(EXIT_UPDATE_STOP) | Some(EXIT_CHANNEL_END) => {
                    self.restart_at
                        .push((surface.monitor.clone(), now + Duration::from_secs(1)));
                }
                _ => {
                    if let Some(code) = self.crashed(&surface.monitor) {
                        return Some(code);
                    }
                }
            }
        }
        None
    }

    /// Records a crash of the surface on `monitor`; `Some(code)` after a halt.
    fn crashed(&mut self, monitor: &str) -> Option<i32> {
        match self.crashes.record(Instant::now()) {
            CrashVerdict::RestartAfter(delay) => {
                self.dir.log(
                    "surface_restart",
                    &format!("{monitor} in {} s", delay.as_secs()),
                );
                self.restart_at
                    .push((monitor.to_string(), Instant::now() + delay));
                None
            }
            CrashVerdict::Halt => Some(super::supervisor_surface::halt(self.halt_parts())),
        }
    }

    fn halt_parts(&mut self) -> super::supervisor_surface::HaltParts<'_> {
        super::supervisor_surface::HaltParts {
            dir: &self.dir,
            surfaces: &mut self.surfaces,
            update: &self.update,
            restart: &self.restart,
        }
    }

    fn push_models(&mut self) {
        let Some(feed) = self.feed.latest() else {
            return;
        };
        if feed.rev != self.feed_rev {
            self.feed_rev = feed.rev;
            self.model_rev += 1;
            layout_store::forget_missing_files(&mut self.layout, &feed.files);
        }
        let config = self
            .config
            .clone()
            .unwrap_or_else(|| DesktopConfig::initial(platform::initial_system_items()));
        let items = crate::desktop::core::items::surface_items(
            platform::PLATFORM,
            &config,
            &feed.files,
            &feed.tiles,
        );
        if let Some(primary) = self.primary_monitor() {
            if self
                .layout
                .place_new_items(&primary, &items)
                .is_ok_and(|new| !new.is_empty())
            {
                self.model_rev += 1;
            }
        }
        let rev = self.model_rev;
        for surface in self.surfaces.iter_mut().filter(|surface| surface.greeted) {
            if surface.sent_rev == Some(rev) {
                continue;
            }
            let Some(model) = super::supervisor_surface::surface_model(
                &feed.tiles,
                &feed.files,
                &config,
                &mut self.layout,
                &surface.monitor,
            ) else {
                continue;
            };
            if surface.send(ShellMessage::Model {
                rev,
                model: Box::new(model),
            }) {
                surface.sent_rev = Some(rev);
            }
        }
    }

    fn end_for_update(&mut self) -> i32 {
        self.dir.log("update_stop", "");
        self.end(StopReason::Update, EXIT_UPDATE_STOP, "update")
    }

    fn end(&mut self, reason: StopReason, code: i32, why: &str) -> i32 {
        super::supervisor_surface::stop_all(&self.dir, &mut self.surfaces, reason);
        self.dir.log("exit", &format!("{code} {why}"));
        self.dir.clear_shell();
        code
    }
}
