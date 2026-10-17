//! Types of the transitions of se Desktop (C5.3, C5.7): the platform's
//! registrations ([`Integration`], PowerToys pattern `apply`, `unApply`,
//! `isApplied`, Ref D1), the shell process as seen from outside
//! ([`ShellProcesses`]), and what a transition reports.
use std::fmt;
use std::path::{Path, PathBuf};

use super::store::DesktopDir;
use crate::desktop::core::config::ConfigError;
use crate::desktop::core::ledger::{LedgerEntry, Registration, Trace};
use crate::desktop::core::state::{Part, Platform, State, StateError};

/// The shell's login start as the status reports it (C5.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutostartState {
    On,
    Off,
    /// The user switched the entry off in the system (Task Manager,
    /// `Hidden=true`); se reports it and never overwrites it.
    DisabledBySystem,
}

impl AutostartState {
    pub(crate) fn key(self) -> &'static str {
        match self {
            AutostartState::On => "on",
            AutostartState::Off => "off",
            AutostartState::DisabledBySystem => "disabled_by_system",
        }
    }
}

/// Registrations of the platform. Only fixed names of this module are ever
/// written or removed; foreign values under the same keys stay untouched.
pub(crate) trait Integration {
    fn platform(&self) -> Platform;
    /// The registration that starts the shell at login.
    fn autostart_registration(&self) -> Registration;
    /// Applies `registration` for the GUI program `gui`; returns its ledger entry.
    fn apply(&mut self, registration: Registration, gui: &Path) -> Result<LedgerEntry, String>;
    /// Removes one trace; a trace that does not exist is removed.
    fn unapply(&mut self, trace: &Trace) -> Result<(), String>;
    /// Whether the trace exists now (never knowable for the jump list, A6).
    fn is_applied(&self, trace: &Trace) -> bool;
    fn autostart(&self) -> AutostartState;
    /// Runtime files of this module that exist now (Linux, C5.7).
    fn runtime_files(&self) -> Vec<PathBuf>;
}

/// Outcome of stopping the shell from another process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StopReport {
    /// No shell instance runs any more.
    pub(crate) stopped: bool,
    /// Processes ended by PID after their image was checked.
    pub(crate) killed: Vec<u32>,
    pub(crate) error: Option<String>,
}

/// The shell process as seen from `se desktop` and the settings.
pub(crate) trait ShellProcesses {
    fn running(&self, dir: &DesktopDir) -> bool;
    /// Starts `"<GUI>" --desktop-shell` detached from the caller.
    fn start(&self, gui: &Path) -> Result<(), String>;
    /// Waits until the shell ended after a stop request (state or
    /// `shell.stop`); then ends what remains by PID after checking its image.
    fn await_stop(&self, dir: &DesktopDir, gui: Option<&Path>) -> StopReport;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ControlError {
    /// No Smart Explorer app beside this se (or the reason it is unknown).
    NoApp(String),
    PartUnavailable {
        part: Part,
        reason: String,
    },
    State(StateError),
    Config(ConfigError),
    Io(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlError::NoApp(why) => formatter.write_str(why),
            ControlError::PartUnavailable { part, reason } => write!(
                formatter,
                "Teil {} ist hier nicht verfügbar: {reason}",
                part.cli_name()
            ),
            ControlError::State(error) => write!(formatter, "{error}"),
            ControlError::Config(error) => write!(formatter, "{error}"),
            ControlError::Io(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<StateError> for ControlError {
    fn from(error: StateError) -> ControlError {
        match error {
            StateError::PartUnavailable(part) => ControlError::PartUnavailable {
                part,
                reason: "in dieser Sitzung nicht verfügbar".into(),
            },
            other => ControlError::State(other),
        }
    }
}

/// What a transition did besides the new state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// Traces that still exist after `disable`/`remove` (`residue`).
    pub(crate) residue: Vec<Trace>,
    /// Runtime files that could not be removed.
    pub(crate) residue_files: Vec<PathBuf>,
    /// Removals and starts that failed, with reason.
    pub(crate) errors: Vec<String>,
    pub(crate) started: bool,
    pub(crate) stop: Option<StopReport>,
    /// `state.json` could not be read; `remove` still deletes the folder.
    pub(crate) state_error: Option<String>,
}

impl Outcome {
    pub(crate) fn complete(&self) -> bool {
        self.residue.is_empty() && self.residue_files.is_empty() && self.errors.is_empty()
    }
}

/// `se desktop restart` (`{"restarted","state","reason"}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestartOutcome {
    pub(crate) restarted: bool,
    pub(crate) state: State,
    pub(crate) reason: Option<String>,
    /// The shell should run but could not be stopped or started.
    pub(crate) failed: bool,
}
