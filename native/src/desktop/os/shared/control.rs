//! Transitions of se Desktop (C5.3, C5.6, C5.7) with the registration
//! ledger: enable, disable, remove, part, config, restart, and the removal of
//! every system trace for the uninstaller. The platform applies and removes
//! its registrations through [`Integration`] (PowerToys pattern `apply`,
//! `unApply`, `isApplied`, Ref D1); the shell process is started, stopped and
//! probed through [`ShellProcesses`]. One registration, the shell's login
//! start, serves every part; the Windows parts `nav_pane` and `jump_list` are
//! written by the shell (DSK-M5) and removed here like every other trace.
use std::path::{Path, PathBuf};

pub(crate) use super::control_types::{
    AutostartState, ControlError, Integration, Outcome, RestartOutcome, ShellProcesses, StopReport,
};
use super::store::DesktopDir;
use crate::desktop::core::config::{
    apply_change, ConfigChange, ConfigContext, DesktopConfig, SystemItems,
};
use crate::desktop::core::ledger::{removal_plan, Registration, Trace};
use crate::desktop::core::state::{self, ModuleState, Part, PartSet, State, StateError};

/// One control call: the folder, the platform and the facts of the session.
pub(crate) struct Control<'a> {
    pub(crate) dir: DesktopDir,
    pub(crate) integration: &'a mut dyn Integration,
    pub(crate) processes: &'a dyn ShellProcesses,
    /// GUI program beside this se, or why there is none.
    pub(crate) gui: Result<PathBuf, String>,
    /// Parts the session offers, with the reason for each missing one.
    pub(crate) available: PartSet,
    pub(crate) unavailable: Vec<(Part, String)>,
    pub(crate) system_items: SystemItems,
    pub(crate) version: String,
    pub(crate) now: i64,
}

fn io(error: String) -> ControlError {
    ControlError::Io(error)
}

impl Control<'_> {
    fn gui(&self) -> Result<PathBuf, ControlError> {
        self.gui.clone().map_err(ControlError::NoApp)
    }

    fn reason_for(&self, part: Part) -> String {
        self.unavailable
            .iter()
            .find(|(missing, _)| *missing == part)
            .map(|(_, reason)| reason.clone())
            .unwrap_or_else(|| "in dieser Sitzung nicht verfügbar".into())
    }

    fn autostart_trace(&self) -> Trace {
        let kind = self.integration.autostart_registration();
        let target = crate::desktop::core::ledger::KNOWN_TRACES
            .iter()
            .find(|(known, _)| *known == kind)
            .map(|(_, name)| (*name).to_string())
            .unwrap_or_default();
        Trace { kind, target }
    }

    /// The login start exists exactly while the module is enabled with a
    /// part on (C5.6).
    fn sync_autostart(&mut self, next: &mut ModuleState, gui: Option<&Path>) -> Result<(), String> {
        let wanted = next.state == State::Enabled && next.parts.any();
        let trace = self.autostart_trace();
        match (wanted, gui) {
            (true, Some(gui)) => {
                let entry = self
                    .integration
                    .apply(trace.kind, gui)
                    .map_err(|error| format!("Autostart von se Desktop: {error}"))?;
                next.record(entry);
            }
            (true, None) => {}
            (false, _) => {
                self.integration
                    .unapply(&trace)
                    .map_err(|error| format!("Autostart von se Desktop: {error}"))?;
                next.forget(trace.kind, &trace.target);
            }
        }
        Ok(())
    }

    /// `se desktop enable [--part …]`.
    pub(crate) fn enable(&mut self, requested: Option<&[Part]>) -> Result<Outcome, ControlError> {
        let gui = self.gui()?;
        if let Some(part) = requested
            .unwrap_or_default()
            .iter()
            .copied()
            .find(|part| !self.available.contains(*part))
        {
            return Err(ControlError::PartUnavailable {
                part,
                reason: self.reason_for(part),
            });
        }
        let created = !self.dir.exists();
        self.dir.create().map_err(io)?;
        let result = self.enable_locked(requested, &gui);
        if result.is_err() && created {
            let _ = self.dir.delete_all();
        }
        let mut outcome = result?;
        outcome.started = self.start_if_wanted(&gui, &mut outcome.errors);
        Ok(outcome)
    }

    fn enable_locked(
        &mut self,
        requested: Option<&[Part]>,
        gui: &Path,
    ) -> Result<Outcome, ControlError> {
        let _lock = self.dir.lock().map_err(io)?;
        let current = self.dir.read_state().map_err(io)?;
        let mut next = state::enable(current.as_ref(), requested, self.available, self.now)?;
        if self.dir.read_config().map_err(io)?.is_none() {
            let config = DesktopConfig::initial(self.system_items);
            self.dir.write_config(&config).map_err(io)?;
        }
        self.sync_autostart(&mut next, Some(gui)).map_err(io)?;
        self.dir.write_state(&next).map_err(io)?;
        Ok(Outcome::default())
    }

    fn start_if_wanted(&self, gui: &Path, errors: &mut Vec<String>) -> bool {
        let Ok(Some(current)) = self.dir.read_state() else {
            return false;
        };
        if !current.should_resume(&self.version) || self.processes.running(&self.dir) {
            return false;
        }
        match self.processes.start(gui) {
            Ok(()) => true,
            Err(error) => {
                errors.push(format!("se Desktop starten: {error}"));
                false
            }
        }
    }

    /// `se desktop disable`: stops the processes and removes every trace;
    /// configuration and arrangement stay (C5.7).
    pub(crate) fn disable(&mut self) -> Result<Outcome, ControlError> {
        let mut outcome = Outcome::default();
        if self.dir.exists() {
            {
                let _lock = self.dir.lock().map_err(io)?;
                match self.dir.read_state() {
                    Ok(current) => {
                        if let Some(next) = state::disable(current.as_ref(), self.now) {
                            self.dir.write_state(&next).map_err(io)?;
                        }
                    }
                    // A broken file must not block disabling or removing.
                    Err(error) => outcome.state_error = Some(error),
                }
            }
            let report = self.stop_shell();
            if !report.stopped {
                outcome.errors.push(
                    report
                        .error
                        .clone()
                        .unwrap_or_else(|| "se Desktop läuft noch".into()),
                );
            }
            outcome.stop = Some(report);
        }
        self.remove_traces(&mut outcome)?;
        Ok(outcome)
    }

    /// `se desktop remove`: like `disable`, then the folder with state,
    /// configuration and arrangement. A failed removal keeps the folder, so
    /// its ledger allows a later retry.
    pub(crate) fn remove(&mut self) -> Result<Outcome, ControlError> {
        let mut outcome = self.disable()?;
        if outcome.complete() {
            self.dir.delete_all().map_err(io)?;
        }
        Ok(outcome)
    }

    fn stop_shell(&self) -> StopReport {
        if !self.processes.running(&self.dir) {
            return StopReport {
                stopped: true,
                ..StopReport::default()
            };
        }
        self.processes
            .await_stop(&self.dir, self.gui.as_deref().ok())
    }

    /// Every fixed name and every ledger entry of the platform, and the
    /// runtime files; removed entries leave the ledger.
    fn remove_traces(&mut self, outcome: &mut Outcome) -> Result<(), ControlError> {
        let platform = self.integration.platform();
        let lock = if self.dir.exists() {
            Some(self.dir.lock().map_err(io)?)
        } else {
            None
        };
        let mut current = self.dir.read_state().unwrap_or(None);
        let ledger = current
            .as_ref()
            .map(|module| module.ledger.clone())
            .unwrap_or_default();
        for trace in removal_plan(platform, &ledger) {
            match self.integration.unapply(&trace) {
                Ok(()) => {
                    if let Some(module) = current.as_mut() {
                        module.forget(trace.kind, &trace.target);
                    }
                }
                Err(error) => {
                    outcome
                        .errors
                        .push(format!("{} {}: {error}", trace.kind.key(), trace.target));
                }
            }
        }
        for file in self.integration.runtime_files() {
            if let Err(error) = std::fs::remove_file(&file) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    outcome.errors.push(format!("{}: {error}", file.display()));
                }
            }
        }
        if let Some(module) = &current {
            self.dir.write_state(module).map_err(io)?;
        }
        drop(lock);
        let left = current.map(|module| module.ledger).unwrap_or_default();
        outcome.residue = removal_plan(platform, &left)
            .into_iter()
            .filter(|trace| self.integration.is_applied(trace))
            .collect();
        outcome.residue_files = self.integration.runtime_files();
        Ok(())
    }

    /// `se desktop part <TEIL> on|off`.
    pub(crate) fn set_part(&mut self, part: Part, on: bool) -> Result<Outcome, ControlError> {
        if on && !self.available.contains(part) {
            return Err(ControlError::PartUnavailable {
                part,
                reason: self.reason_for(part),
            });
        }
        let gui = if on { Some(self.gui()?) } else { None };
        if !self.dir.exists() {
            return Err(ControlError::State(StateError::NotAdded));
        }
        let mut outcome = Outcome::default();
        {
            let _lock = self.dir.lock().map_err(io)?;
            let current = self.dir.read_state().map_err(io)?;
            let mut next = state::set_part(current.as_ref(), part, on, self.available, self.now)?;
            self.sync_autostart(&mut next, gui.as_deref()).map_err(io)?;
            if !on && part != Part::Surface {
                // Windows parts the shell maintains go at once (C5.7).
                let kind = match part {
                    Part::NavPane => Registration::NavPaneClsid,
                    _ => Registration::JumpList,
                };
                let plan = removal_plan(self.integration.platform(), &next.ledger);
                for trace in plan.into_iter().filter(|trace| trace.kind == kind) {
                    match self.integration.unapply(&trace) {
                        Ok(()) => next.forget(trace.kind, &trace.target),
                        Err(error) => outcome.errors.push(format!("{}: {error}", kind.key())),
                    }
                }
            }
            self.dir.write_state(&next).map_err(io)?;
        }
        if let Some(gui) = gui {
            outcome.started = self.start_if_wanted(&gui, &mut outcome.errors);
        }
        Ok(outcome)
    }

    /// `se desktop config …`: checked against the session, then stored; the
    /// shell reads the file again within a second.
    pub(crate) fn configure(
        &mut self,
        change: &ConfigChange,
        context: &ConfigContext,
    ) -> Result<DesktopConfig, ControlError> {
        if self.dir.read_state().map_err(io)?.is_none() {
            return Err(ControlError::State(StateError::NotAdded));
        }
        let _lock = self.dir.lock().map_err(io)?;
        let config = self
            .dir
            .read_config()
            .map_err(io)?
            .unwrap_or_else(|| DesktopConfig::initial(self.system_items));
        let next = apply_change(&config, change, context).map_err(ControlError::Config)?;
        if next != config {
            self.dir.write_config(&next).map_err(io)?;
        }
        Ok(next)
    }

    /// `se desktop restart` and the button "Neu starten": clears a halt,
    /// stops a running shell and starts it again when the module is enabled
    /// with a part on. The installers call it after every installation.
    pub(crate) fn restart(&mut self) -> Result<RestartOutcome, ControlError> {
        let gui = self.gui()?;
        let Some(mut module) = self.dir.read_state().map_err(io)? else {
            return Ok(RestartOutcome {
                restarted: false,
                state: State::Absent,
                reason: Some(StateError::NotAdded.to_string()),
                failed: false,
            });
        };
        if module.halted.is_some() {
            let _lock = self.dir.lock().map_err(io)?;
            if let Some(mut fresh) = self.dir.read_state().map_err(io)? {
                fresh.clear_halt();
                self.dir.write_state(&fresh).map_err(io)?;
                module = fresh;
            }
        }
        if self.processes.running(&self.dir) {
            self.dir.request_stop().map_err(io)?;
            let report = self.processes.await_stop(&self.dir, Some(gui.as_path()));
            let _ = std::fs::remove_file(self.dir.stop_path());
            if !report.stopped {
                return Ok(RestartOutcome {
                    restarted: false,
                    state: module.state,
                    reason: Some(report.error.unwrap_or_else(|| {
                        "der laufende se Desktop hat sich nicht beendet".into()
                    })),
                    failed: true,
                });
            }
        }
        let _ = std::fs::remove_file(self.dir.stop_path());
        if !module.should_resume(&self.version) {
            let reason = if module.state != State::Enabled {
                "se Desktop ist deaktiviert"
            } else {
                "kein Teil ist eingeschaltet"
            };
            return Ok(RestartOutcome {
                restarted: false,
                state: module.state,
                reason: Some(reason.into()),
                failed: false,
            });
        }
        match self.processes.start(&gui) {
            Ok(()) => Ok(RestartOutcome {
                restarted: true,
                state: module.state,
                reason: None,
                failed: false,
            }),
            Err(error) => Ok(RestartOutcome {
                restarted: false,
                state: module.state,
                reason: Some(format!("se Desktop starten: {error}")),
                failed: true,
            }),
        }
    }

    /// `--unregister` of the uninstaller (GUI image): every system trace
    /// again, also after a failed `se desktop remove`; an enabled module is
    /// left disabled so that a later installation does not revive it.
    pub(crate) fn remove_system_traces(&mut self) -> Outcome {
        let mut outcome = Outcome::default();
        if self.dir.exists() {
            if let Ok(_lock) = self.dir.lock() {
                if let Ok(Some(module)) = self.dir.read_state() {
                    if let Some(next) = state::disable(Some(&module), self.now) {
                        if let Err(error) = self.dir.write_state(&next) {
                            outcome.errors.push(error);
                        }
                    }
                }
            }
        }
        if let Err(error) = self.remove_traces(&mut outcome) {
            outcome.errors.push(error.to_string());
        }
        outcome
    }
}
