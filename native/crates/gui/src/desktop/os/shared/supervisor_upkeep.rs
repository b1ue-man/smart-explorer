//! The shell's rounds besides its surfaces (C1.2, C5.5, DSK-M5, DSK-M6),
//! part of `supervisor.rs`: the mechanism and the monitor list (decided at
//! the start, on a changed configuration and monitor list, every five
//! seconds while the GNOME Shell version is unknown and shortly after a
//! foreign extension changed); once a second the monitors are compared with
//! the list the surfaces started with, the GNOME extension's files follow
//! the mechanism, the platform's other parts follow `state.json` and the
//! model (Windows: navigation pane and jump list, `ShellParts`), and
//! `shell.json` tells status and stop what runs.
//!
//! A changed monitor list (keys, rectangles, primary monitor) restarts every
//! running surface: the one whose monitor changed reads its rectangle and
//! scale anew (the shell is not DPI aware, so a new scale shows as a new
//! rectangle here, C1.2), new monitors get surfaces, those of a monitor that
//! left end, and every other surface gets the new list for "Auf Bildschirm
//! verschieben" and dragging between monitors. Under the suite switch
//! `SE_DESKTOP_TEST_MONITORS` the list is only read again.
use std::time::{Duration, Instant};

use super::Shell;
use crate::desktop::core::config::ForeignDesktopIcons;
use crate::desktop::core::mechanism::{self, Mechanism, Reason};
use crate::desktop::core::protocol::StopReason;
use crate::desktop::core::tiles::ShareState;
use crate::desktop::os::feed_sources::DeviceTreeView;
use crate::desktop::os::platform;
use crate::desktop::os::store::{ShellRuntime, SurfaceRuntime};
use crate::desktop::os::Monitor;

/// While the GNOME Shell's version is unknown (the Shell may not be on the
/// bus yet at login) the facts are collected again this often.
const GNOME_RECHECK: Duration = Duration::from_secs(5);

fn sorted(mut monitors: Vec<Monitor>) -> Vec<Monitor> {
    monitors.sort_by(|left, right| left.key.cmp(&right.key));
    monitors
}

impl Shell {
    /// Once a second, after `state.json` and `config.json` were read.
    pub(super) fn upkeep(&mut self) {
        self.poll_monitors();
        self.redecide_if_due();
        self.sync_gnome();
        self.sync_parts();
    }

    /// The mechanism of the session as it is now.
    pub(super) fn decide_now(&mut self) {
        let icons = self
            .config
            .as_ref()
            .map_or(ForeignDesktopIcons::Keep, |config| {
                config.foreign_desktop_icons
            });
        self.mechanism = mechanism::decide(&platform::session_facts(), icons);
        self.decided_at = Instant::now();
    }

    fn redecide_if_due(&mut self) {
        let unknown = matches!(
            self.mechanism,
            Mechanism::Unavailable(Reason::GnomeVersionUnknown)
        );
        let settled = self.recheck_at.is_some_and(|at| Instant::now() >= at);
        if (unknown && self.decided_at.elapsed() >= GNOME_RECHECK) || settled {
            self.recheck_at = None;
            self.decide_now();
            self.model_rev += 1;
        }
    }

    pub(super) fn refresh_monitors(&mut self) {
        self.monitors = platform::monitors();
        self.decide_now();
        self.model_rev += 1;
        // C5.5: the primary monitor keeps the items of a monitor that left.
        if let Some(primary) = self.primary_monitor() {
            let present = self.shown_monitors();
            let _ = self.layout.adopt_removed_monitors(&primary, &present);
        }
    }

    /// Surfaces that get a process now (`--monitor <key>` each, or the one
    /// `--layer` process, DSK-M7).
    pub(super) fn wanted_monitors(&self) -> Vec<String> {
        super::super::supervisor_surface::wanted_surfaces(
            &self.mechanism,
            self.surface_on,
            &self.monitors,
            self.test_monitors.as_deref(),
        )
    }

    /// Monitors whose arrangement is shown: the layer process serves every
    /// output of the session and the GNOME extension every monitor, so there
    /// it is the session's monitor list.
    pub(super) fn shown_monitors(&self) -> Vec<String> {
        let one_process_for_all = self.surface_on
            && self.test_monitors.is_none()
            && matches!(
                self.mechanism,
                Mechanism::LayerShell { .. } | Mechanism::GnomeExtension { .. }
            );
        if one_process_for_all {
            self.monitors
                .iter()
                .map(|monitor| monitor.key.clone())
                .collect()
        } else {
            self.wanted_monitors()
        }
    }

    pub(super) fn primary_monitor(&self) -> Option<String> {
        let shown = self.shown_monitors();
        self.monitors
            .iter()
            .find(|monitor| monitor.primary && shown.contains(&monitor.key))
            .map(|monitor| monitor.key.clone())
            .or_else(|| shown.first().cloned())
    }

    /// Restarts the surfaces when the monitors changed (also on a surface's
    /// `monitors_changed`).
    pub(super) fn poll_monitors(&mut self) {
        let now = sorted(platform::monitors());
        if now == sorted(self.monitors.clone()) {
            return;
        }
        self.dir
            .log("monitors_changed", &format!("{} Bildschirme", now.len()));
        if self.test_monitors.is_none() {
            for surface in self
                .surfaces
                .iter_mut()
                .filter(|surface| surface.stopping.is_none())
            {
                surface.stop(StopReason::Restart);
            }
        }
        self.refresh_monitors();
    }

    /// The platform's parts besides the surfaces, after the state read in
    /// this round.
    fn sync_parts(&mut self) {
        let Some(state) = &self.seen else {
            return;
        };
        let feed = self.feed.latest();
        let not_running = DeviceTreeView::NotRunning;
        let tree = feed.as_ref().map_or(&not_running, |feed| &feed.device_tree);
        self.parts
            .sync(state, tree, feed.as_ref().map(|feed| &feed.tiles));
    }

    /// `shell.json`, written when it changed.
    pub(super) fn write_runtime(&mut self) {
        let (share_state, reason) = match self.feed.latest().map(|feed| feed.tiles.share_state) {
            Some(ShareState::Live) => ("live", None),
            Some(ShareState::Profiles { reason }) => ("profiles", Some(reason)),
            None => (
                "profiles",
                Some(crate::desktop::os::model_feed::NO_SERVICE.to_string()),
            ),
        };
        // The layer process is one process for every output: once it has
        // greeted, each output it serves counts as a running surface.
        let shown = self.shown_monitors();
        let mut running = Vec::new();
        for surface in &self.surfaces {
            let served = surface.monitor == super::super::supervisor_surface::LAYER_KEY
                && surface.greeted
                && !shown.is_empty();
            let keys = if served {
                shown.clone()
            } else {
                vec![surface.monitor.clone()]
            };
            running.extend(keys.into_iter().map(|monitor| SurfaceRuntime {
                monitor,
                pid: surface.pid,
            }));
        }
        let runtime = ShellRuntime {
            shell_pid: std::process::id(),
            exe: self.spawn.gui().to_string_lossy().into_owned(),
            surfaces: running,
            monitors: shown.len(),
            share_state: share_state.into(),
            share_state_reason: reason,
        };
        if self.runtime.as_ref() != Some(&runtime) {
            if let Err(error) = self.dir.write_shell(&runtime) {
                self.dir.log("runtime_unwritable", &error);
            }
            self.runtime = Some(runtime);
        }
    }
}
