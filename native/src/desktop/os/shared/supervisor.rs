//! The se Desktop shell `--desktop-shell` (C5.5, C5.6, C9): a windowless
//! process of the GUI image, one per user. It starts one surface per monitor,
//! is their only data holder, watches their heartbeats and restarts them
//! (`crash_rule`), follows `state.json` (disabled or removed: end within a
//! second), `shell.stop` (restart) and the update stop marker (V8: stop the
//! surfaces, end with 75). It never starts the background service (V11).
//! Once a second it also looks at the monitors and keeps the platform's
//! other parts current (DSK-M5 Windows: navigation pane and jump list,
//! `supervisor_upkeep.rs`). Under GNOME the extension starts the surfaces;
//! the shell takes those that connect over the socket (`supervisor_gnome.rs`,
//! DSK-M8). A layer surface refused by the compositor is started again after
//! a growing wait (`supervisor_backoff.rs`).
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

#[path = "supervisor_backoff.rs"]
mod backoff;
#[path = "supervisor_gnome.rs"]
pub(crate) mod gnome;
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
        gnome: gnome::Bridge::new(dir.clone(), gui.clone()),
        backoffs: backoff::Backoffs::default(),
        decided_at: Instant::now(),
        recheck_at: None,
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
    // Every way out: no control file, token or socket of the GNOME channel.
    shell.gnome.finish();
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
    /// The GNOME extension's side of the shell (DSK-M8).
    gnome: gnome::Bridge,
    /// The waits after the end of a surface's channel.
    backoffs: backoff::Backoffs,
    /// When the mechanism was decided last, and when to decide it again.
    decided_at: Instant,
    recheck_at: Option<Instant>,
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
            self.adopt_remote();
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
            self.decide_now();
            if let Some(config) = &self.config {
                for surface in self.surfaces.iter().filter(|surface| surface.greeted) {
                    surface.send(ShellMessage::Config {
                        config: config.clone(),
                    });
                }
            }
        }
    }

    /// Starts the missing surfaces; `Some(code)` when start failures halted
    /// the shell.
    fn ensure_surfaces(&mut self) -> Option<i32> {
        let wanted = self.wanted_monitors();
        for surface in &mut self.surfaces {
            // A surface of the GNOME extension is not started by the shell.
            let ours = !surface.is_remote();
            if ours && !wanted.contains(&surface.monitor) && surface.stopping.is_none() {
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
            let started = if monitor == super::supervisor_surface::LAYER_KEY {
                self.spawn.start_layer()
            } else {
                self.spawn.start(&monitor)
            };
            match started {
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
        let message = match event {
            LinkEvent::Message(message) => message,
            LinkEvent::Ended(_) => {
                // A process is reaped; a surface of the GNOME extension has
                // no process of ours, its connection tells.
                self.surfaces[index].mark_link_closed();
                return;
            }
        };
        match message {
            hello @ SurfaceMessage::Hello { .. } => {
                let expected = self.surfaces[index].monitor.clone();
                if let Err(error) = check_hello(&hello, Some(expected.as_str()), None) {
                    self.dir.log("surface_hello_rejected", &error.to_string());
                    self.surfaces[index].kill();
                    return;
                }
                self.greet(index);
            }
            SurfaceMessage::Alive => self.surfaces[index].last_alive = Instant::now(),
            // The operations carry their monitor; the frame's own `monitor`
            // (a layer process, DSK-M7) is only the origin.
            SurfaceMessage::Layout { op, .. } => {
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

    /// A surface that greeted (or connected over the GNOME socket): it gets
    /// the configuration and, with the next round, the model.
    fn greet(&mut self, index: usize) {
        let surface = &mut self.surfaces[index];
        surface.greeted = true;
        surface.last_alive = Instant::now();
        surface.sent_rev = None;
        if let Some(config) = &self.config {
            surface.send(ShellMessage::Config {
                config: config.clone(),
            });
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
            // The extension restarts its own surfaces and counts their
            // crashes (U34-15).
            if surface.stopping.is_some() || surface.is_remote() {
                continue;
            }
            match code {
                // Not a crash (C5.5): on an update the shell sees the marker
                // within a second and ends first.
                Some(EXIT_UPDATE_STOP) => {
                    self.restart_at
                        .push((surface.monitor.clone(), now + Duration::from_secs(1)));
                }
                // Not a crash either, but a compositor that refuses the
                // surface for good ends it with 76 again and again: the wait
                // grows (1, 2, 4, 8, 16, 32, then 60 seconds) and starts
                // over once a surface ran again.
                Some(EXIT_CHANNEL_END) => {
                    let wait = self
                        .backoffs
                        .next(&surface.monitor, surface.started.elapsed());
                    self.dir.log(
                        "surface_restart",
                        &format!("{} in {} s (Kanalende)", surface.monitor, wait.as_secs()),
                    );
                    self.restart_at.push((surface.monitor.clone(), now + wait));
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
        super::supervisor_surface::send_models(
            &mut self.surfaces,
            &self.monitors,
            self.model_rev,
            (&feed.tiles, &feed.files, &config),
            &mut self.layout,
        );
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
