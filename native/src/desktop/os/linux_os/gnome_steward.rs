//! The shell's side of the GNOME extension (U34-12 to U34-17): while the
//! mechanism is the extension and the surface part is on, it keeps the
//! extension installed (`gnome_extension`), listens on `desktop.sock`
//! (`gnome_channel`), writes the token and `desktop-gnome.json`, and hands the
//! accepted surfaces to the shell. Without the mechanism it takes the control
//! file away, which makes the extension end its surfaces.
//!
//! The control file's generation starts at the time of the shell's start and
//! grows by one with every change of the monitor list: a shell that crashed
//! without removing the file still gives the next one a higher number, so the
//! extension starts the surfaces anew. A foreign desktop-icons extension is
//! switched off only with `foreign_desktop_icons = take_over` and then comes
//! back with `keep`, `disable` and `remove`, each with its ledger entry.
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::gnome_channel::{GnomeListener, ListenerEvent};
use super::gnome_control::{Control, ControlMonitor, RuntimeFiles, PROTOCOL};
use super::gnome_dbus::{SessionBus, ShellBus, STATE_ENABLED};
use super::gnome_extension::{install, take_over, InstallOutcome};
use super::gnome_settings::GnomeSettings;
use super::integration::{LinuxIntegration, LinuxPaths};
use super::session_facts::DESKTOP_ICON_EXTENSIONS;
use crate::desktop::core::config::ForeignDesktopIcons;
use crate::desktop::core::ledger::{LedgerEntry, Registration, Trace};
use crate::desktop::core::mechanism::{GnomeVariant, Mechanism};
use crate::desktop::os::control::Integration;
use crate::desktop::os::store::{unix_now, DesktopDir};
use crate::desktop::os::surface_link::RemoteEnd;
use crate::desktop::os::Monitor;

/// The installation is checked again this often (files, lists).
const HEAL_EVERY: Duration = Duration::from_secs(60);

/// What the shell tells every second.
pub(crate) struct Wanted<'a> {
    pub(crate) mechanism: &'a Mechanism,
    pub(crate) surface_on: bool,
    pub(crate) monitors: &'a [Monitor],
    pub(crate) icons: ForeignDesktopIcons,
}

/// What a round did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Round {
    /// The extension runs the surfaces now.
    pub(crate) active: bool,
    /// A foreign extension was switched off or on: the session's facts
    /// changed and the mechanism is to be decided again.
    pub(crate) session_changed: bool,
}

/// A surface connection ready for the shell, or a refusal to log.
pub(crate) enum Arrival {
    Surface { key: String, end: RemoteEnd },
    Rejected(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Applied {
    variant: GnomeVariant,
    take_over: bool,
}

pub(crate) struct GnomeSteward {
    dir: DesktopDir,
    gui: PathBuf,
    files: Option<RuntimeFiles>,
    paths: LinuxPaths,
    gnome: GnomeSettings,
    bus: Arc<dyn ShellBus>,
    listener: Option<GnomeListener>,
    generation: u64,
    published: Option<Vec<ControlMonitor>>,
    applied: Option<Applied>,
    checked: Option<Instant>,
}

impl GnomeSteward {
    pub(crate) fn new(dir: DesktopDir, gui: PathBuf) -> GnomeSteward {
        GnomeSteward::with(
            dir,
            gui,
            RuntimeFiles::from_env(),
            LinuxPaths::from_env(),
            GnomeSettings::system(),
            Arc::new(SessionBus),
        )
    }

    pub(crate) fn with(
        dir: DesktopDir,
        gui: PathBuf,
        files: Option<RuntimeFiles>,
        paths: LinuxPaths,
        gnome: GnomeSettings,
        bus: Arc<dyn ShellBus>,
    ) -> GnomeSteward {
        GnomeSteward {
            dir,
            gui,
            files,
            paths,
            gnome,
            bus,
            listener: None,
            generation: u64::try_from(unix_now()).unwrap_or(0),
            published: None,
            applied: None,
            checked: None,
        }
    }

    /// Once a second.
    pub(crate) fn sync(&mut self, wanted: &Wanted<'_>) -> Round {
        let variant = match wanted.mechanism {
            Mechanism::GnomeExtension { variant } if wanted.surface_on => Some(*variant),
            _ => None,
        };
        let take_over = wanted.icons == ForeignDesktopIcons::TakeOver;
        let mut round = Round::default();
        let Some(variant) = variant else {
            self.unpublish();
            round.session_changed = !take_over && self.restore_foreign();
            return round;
        };
        round.active = true;
        if !self.open_channel() {
            round.active = false;
            return round;
        }
        let key = Applied { variant, take_over };
        let due =
            self.applied != Some(key) || self.checked.is_none_or(|at| at.elapsed() >= HEAL_EVERY);
        if due {
            round.session_changed = self.ensure(key);
        }
        if !take_over {
            round.session_changed |= self.restore_foreign();
        }
        self.publish(wanted.monitors);
        round
    }

    /// The token and the listener, once per start of the shell.
    fn open_channel(&mut self) -> bool {
        if self.listener.is_some() {
            return true;
        }
        let Some(files) = &self.files else {
            self.dir.log("gnome_channel", "XDG_RUNTIME_DIR fehlt");
            return false;
        };
        if let Err(error) = files.new_token() {
            self.dir.log("gnome_channel", &error);
            return false;
        }
        match GnomeListener::bind(files) {
            Ok(listener) => {
                self.listener = Some(listener);
                true
            }
            Err(error) => {
                self.dir
                    .log("gnome_channel", &format!("desktop.sock: {error}"));
                false
            }
        }
    }

    /// Installs the extension and, with the consent, takes the foreign one
    /// over; `true` when a foreign extension was switched off.
    fn ensure(&mut self, key: Applied) -> bool {
        let first = self.applied != Some(key);
        self.checked = Some(Instant::now());
        self.applied = Some(key);
        match install(&self.paths, &self.gnome, &*self.bus, key.variant) {
            Ok(report) => {
                for entry in report.entries {
                    self.record(entry);
                }
                if first {
                    self.dir
                        .log("gnome_extension", &format!("{:?}", report.outcome));
                }
                if report.outcome == InstallOutcome::NoSettings {
                    self.dir
                        .log("gnome_extension", "gsettings fehlt: nur die Dateien liegen");
                }
            }
            Err(error) => self.dir.log("gnome_extension_failed", &error),
        }
        first && key.take_over && self.take_over_foreign()
    }

    fn take_over_foreign(&mut self) -> bool {
        let active = self.bus.list_extensions().unwrap_or_default();
        let mut changed = false;
        for uuid in DESKTOP_ICON_EXTENSIONS {
            let on = active
                .iter()
                .any(|(known, state)| known == uuid && *state == STATE_ENABLED);
            if !on {
                continue;
            }
            match take_over(&*self.bus, uuid) {
                Ok(entry) => {
                    self.record(entry);
                    self.dir.log("foreign_extension_disabled", uuid);
                    changed = true;
                }
                Err(error) => self.dir.log("foreign_extension_failed", &error),
            }
        }
        changed
    }

    /// `keep` again: the foreign extension this shell switched off comes
    /// back (only with its ledger entry); `true` when it was switched on.
    fn restore_foreign(&mut self) -> bool {
        let Ok(Some(state)) = self.dir.read_state() else {
            return false;
        };
        let taken: Vec<String> = state
            .ledger
            .iter()
            .filter(|entry| entry.kind == Registration::ForeignExtensionDisabled)
            .map(|entry| entry.target.clone())
            .collect();
        let mut integration = LinuxIntegration::with(
            self.paths.clone(),
            self.gnome.clone(),
            Box::new(self.bus.clone()),
        );
        let mut changed = false;
        for uuid in taken {
            let trace = Trace {
                kind: Registration::ForeignExtensionDisabled,
                target: uuid.clone(),
            };
            match integration.unapply(&trace) {
                Ok(()) => {
                    self.forget(Registration::ForeignExtensionDisabled, &uuid);
                    self.dir.log("foreign_extension_enabled", &uuid);
                    changed = true;
                }
                Err(error) => self.dir.log("foreign_extension_failed", &error),
            }
        }
        changed
    }

    fn record(&self, entry: LedgerEntry) {
        let Ok(_lock) = self.dir.lock() else {
            return;
        };
        if let Ok(Some(mut state)) = self.dir.read_state() {
            let known = state
                .ledger
                .iter()
                .any(|known| known.kind == entry.kind && known.target == entry.target);
            if !known {
                state.record(entry);
                let _ = self.dir.write_state(&state);
            }
        }
    }

    fn forget(&self, kind: Registration, target: &str) {
        let Ok(_lock) = self.dir.lock() else {
            return;
        };
        if let Ok(Some(mut state)) = self.dir.read_state() {
            state.forget(kind, target);
            let _ = self.dir.write_state(&state);
        }
    }

    /// `desktop-gnome.json` with a higher generation when the monitors
    /// changed.
    fn publish(&mut self, monitors: &[Monitor]) {
        let (Some(files), Some(listener)) = (&self.files, &self.listener) else {
            return;
        };
        let list: Vec<ControlMonitor> = monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| {
                let [left, top, right, bottom] = monitor.rect.unwrap_or_default();
                ControlMonitor {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    key: monitor.key.clone(),
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                }
            })
            .collect();
        if self.published.as_ref() == Some(&list) {
            return;
        }
        self.generation += 1;
        let control = Control {
            protocol: PROTOCOL,
            exe: self.gui.to_string_lossy().into_owned(),
            generation: self.generation,
            socket: listener.socket().to_string_lossy().into_owned(),
            monitors: list.clone(),
        };
        match files.write_control(&control) {
            Ok(()) => self.published = Some(list),
            Err(error) => self.dir.log("gnome_control_unwritable", &error),
        }
    }

    fn unpublish(&mut self) {
        if self.published.take().is_some() {
            if let Some(files) = &self.files {
                files.remove_control();
            }
        }
        self.applied = None;
    }

    /// The surfaces that connected and the connections that were refused.
    pub(crate) fn arrivals(&mut self) -> Vec<Arrival> {
        let Some(listener) = &self.listener else {
            return Vec::new();
        };
        let mut arrivals = Vec::new();
        for event in listener.drain() {
            match event {
                ListenerEvent::Rejected(why) => arrivals.push(Arrival::Rejected(why)),
                ListenerEvent::Accepted(accepted) => {
                    let key = self
                        .published
                        .iter()
                        .flatten()
                        .find(|monitor| monitor.index == accepted.index)
                        .map(|monitor| monitor.key.clone());
                    let end = accepted.stream.try_clone().map(|writer| RemoteEnd {
                        reader: Box::new(accepted.stream) as Box<dyn Read + Send>,
                        writer: Box::new(writer) as Box<dyn Write + Send>,
                        pid: accepted.pid,
                    });
                    match (key, end) {
                        (Some(key), Ok(end)) => arrivals.push(Arrival::Surface { key, end }),
                        (None, _) => arrivals.push(Arrival::Rejected(format!(
                            "Monitor gnome-{} steht nicht in desktop-gnome.json",
                            accepted.index
                        ))),
                        (_, Err(error)) => arrivals.push(Arrival::Rejected(error.to_string())),
                    }
                }
            }
        }
        arrivals
    }

    /// The shell's end: no control file, no token, no socket.
    pub(crate) fn finish(&mut self) {
        self.published = None;
        if let Some(files) = &self.files {
            files.remove_shell_files();
        }
        self.listener = None;
    }
}
