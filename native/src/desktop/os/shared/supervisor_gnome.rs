//! The GNOME extension as the shell sees it (DSK-M8), part of
//! `supervisor.rs`: on GNOME the extension starts the surfaces, so the shell
//! writes its control files, listens on the socket and takes the surfaces
//! that connect as its own (U34-12 to U34-15). The work is the Linux
//! adapter's (`linux_os::gnome_steward`); this file is the platform-neutral
//! face of it, a stand-in without any effect on the other platforms, and the
//! `extension_state` of the status.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::super::store::DesktopDir;
use super::super::surface_link::RemoteEnd;
use super::super::Monitor;
use super::Shell;
use crate::desktop::core::config::ForeignDesktopIcons;
use crate::desktop::core::mechanism::{Mechanism, SessionFacts};
use crate::desktop::core::protocol::StopReason;

#[cfg(target_os = "linux")]
use super::super::platform::gnome_steward::{Arrival, GnomeSteward, Wanted};

/// After a foreign extension was switched off or on the facts of the session
/// are read again once, this long afterwards.
const RECHECK_AFTER: Duration = Duration::from_secs(2);

/// What a round of the steward did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Outcome {
    pub(super) active: bool,
    pub(super) session_changed: bool,
}

/// A connection of a surface of the extension.
pub(super) enum Remote {
    Surface { key: String, end: RemoteEnd },
    Rejected(String),
}

/// The steward of the Linux adapter; nothing elsewhere.
pub(super) struct Bridge {
    #[cfg(target_os = "linux")]
    steward: GnomeSteward,
}

impl Bridge {
    #[cfg(target_os = "linux")]
    pub(super) fn new(dir: DesktopDir, gui: PathBuf) -> Bridge {
        Bridge {
            steward: GnomeSteward::new(dir, gui),
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn new(_dir: DesktopDir, _gui: PathBuf) -> Bridge {
        Bridge {}
    }

    #[cfg(target_os = "linux")]
    fn sync(
        &mut self,
        mechanism: &Mechanism,
        surface_on: bool,
        monitors: &[Monitor],
        icons: ForeignDesktopIcons,
    ) -> Outcome {
        let round = self.steward.sync(&Wanted {
            mechanism,
            surface_on,
            monitors,
            icons,
        });
        Outcome {
            active: round.active,
            session_changed: round.session_changed,
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn sync(
        &mut self,
        _mechanism: &Mechanism,
        _surface_on: bool,
        _monitors: &[Monitor],
        _icons: ForeignDesktopIcons,
    ) -> Outcome {
        Outcome::default()
    }

    #[cfg(target_os = "linux")]
    fn arrivals(&mut self) -> Vec<Remote> {
        self.steward
            .arrivals()
            .into_iter()
            .map(|arrival| match arrival {
                Arrival::Surface { key, end } => Remote::Surface { key, end },
                Arrival::Rejected(why) => Remote::Rejected(why),
            })
            .collect()
    }

    #[cfg(not(target_os = "linux"))]
    fn arrivals(&mut self) -> Vec<Remote> {
        Vec::new()
    }

    /// The shell's end: no control file, token or socket remains.
    pub(super) fn finish(&mut self) {
        #[cfg(target_os = "linux")]
        self.steward.finish();
    }
}

/// `extension_state` of the status for the session `facts`; `None` outside
/// GNOME on Wayland and on other platforms.
pub(crate) fn extension_state(facts: &SessionFacts) -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        super::super::platform::extension_state(facts)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = facts;
        None
    }
}

impl Shell {
    /// Once a second: the extension's files and channel follow the mechanism.
    pub(super) fn sync_gnome(&mut self) {
        let icons = self
            .config
            .as_ref()
            .map_or(ForeignDesktopIcons::Keep, |config| {
                config.foreign_desktop_icons
            });
        let outcome = self
            .gnome
            .sync(&self.mechanism, self.surface_on, &self.monitors, icons);
        if !outcome.active {
            // The control file is gone, so the extension ends its surfaces;
            // the shell tells the connected ones as well.
            for surface in self
                .surfaces
                .iter_mut()
                .filter(|surface| surface.is_remote() && surface.stopping.is_none())
            {
                surface.stop(StopReason::Disable);
            }
        }
        if outcome.session_changed {
            self.recheck_at = Some(Instant::now() + RECHECK_AFTER);
        }
    }

    /// Takes the surfaces that connected since the last round; a surface
    /// that comes for a monitor which has one replaces it (the extension
    /// restarted it).
    pub(super) fn adopt_remote(&mut self) {
        for remote in self.gnome.arrivals() {
            let (key, end) = match remote {
                Remote::Surface { key, end } => (key, end),
                Remote::Rejected(why) => {
                    self.dir.log("surface_hello_rejected", &why);
                    continue;
                }
            };
            for old in self
                .surfaces
                .iter_mut()
                .filter(|surface| surface.is_remote() && surface.monitor == key)
            {
                old.kill();
            }
            match self.spawn.adopt_remote(&key, end) {
                Ok(surface) => {
                    self.dir
                        .log("surface_start", &format!("{key} {} gnome", surface.pid));
                    self.surfaces.push(surface);
                    let index = self.surfaces.len() - 1;
                    self.greet(index);
                    self.model_rev += 1;
                }
                Err(error) => self
                    .dir
                    .log("surface_spawn_failed", &format!("{key} {error}")),
            }
        }
    }
}
