//! The shell's rounds besides its surfaces (C1.2, C5.5, DSK-M5), part of
//! `supervisor.rs`: once a second the monitors are compared with the list
//! the surfaces started with, the platform's other parts follow `state.json`
//! and the model (Windows: navigation pane and jump list, `ShellParts`), and
//! `shell.json` tells status and stop what runs.
//!
//! A changed monitor list (keys, rectangles, primary monitor) restarts every
//! running surface: the one whose monitor changed reads its rectangle and
//! scale anew (the shell is not DPI aware, so a new scale shows as a new
//! rectangle here, C1.2), new monitors get surfaces, those of a monitor that
//! left end, and every other surface gets the new list for "Auf Bildschirm
//! verschieben" and dragging between monitors. Under the suite switch
//! `SE_DESKTOP_TEST_MONITORS` the list is only read again.
use super::Shell;
use crate::desktop::core::protocol::StopReason;
use crate::desktop::core::tiles::ShareState;
use crate::desktop::os::feed_sources::DeviceTreeView;
use crate::desktop::os::platform;
use crate::desktop::os::store::{ShellRuntime, SurfaceRuntime};
use crate::desktop::os::Monitor;

fn sorted(mut monitors: Vec<Monitor>) -> Vec<Monitor> {
    monitors.sort_by(|left, right| left.key.cmp(&right.key));
    monitors
}

impl Shell {
    /// Once a second, after `state.json` and `config.json` were read.
    pub(super) fn upkeep(&mut self) {
        self.poll_monitors();
        self.sync_parts();
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
        let runtime = ShellRuntime {
            shell_pid: std::process::id(),
            exe: self.spawn.gui().to_string_lossy().into_owned(),
            surfaces: self
                .surfaces
                .iter()
                .map(|surface| SurfaceRuntime {
                    monitor: surface.monitor.clone(),
                    pid: surface.pid,
                })
                .collect(),
            monitors: self.wanted_monitors().len(),
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
