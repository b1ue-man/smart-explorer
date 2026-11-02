//! Module state of se Desktop (C5.3) and `state.json` schema 1 (C5.4).
//!
//! `absent` → (enable) → `enabled` ⇄ (disable/enable) `disabled` → (remove) →
//! `absent`; `enabled` → (remove) → `absent`. Every transition is idempotent.
//! `absent` is never stored: it means the folder `<AD>/desktop/` is missing.
//! The os layer applies and removes registrations and writes this file
//! atomically; the functions here only decide.
use std::fmt;

use serde::{Deserialize, Serialize};

use super::ledger::{LedgerEntry, Registration};

pub const STATE_SCHEMA: u32 = 1;

/// The platform the os layer runs on; passed in, never detected here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    Windows,
    Linux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Absent,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Enable,
    Disable,
    Remove,
}

impl State {
    pub fn key(self) -> &'static str {
        match self {
            State::Absent => "absent",
            State::Enabled => "enabled",
            State::Disabled => "disabled",
        }
    }

    /// The state machine of C5.3.
    pub fn after(self, action: Action) -> State {
        match (self, action) {
            (_, Action::Remove) => State::Absent,
            (_, Action::Enable) => State::Enabled,
            (State::Absent, Action::Disable) => State::Absent,
            (State::Enabled | State::Disabled, Action::Disable) => State::Disabled,
        }
    }

    pub fn of(current: Option<&ModuleState>) -> State {
        current.map_or(State::Absent, |module| module.state)
    }
}

/// Windows: surface, navigation pane, jump list; Linux: surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    Surface,
    NavPane,
    JumpList,
}

impl Part {
    pub const ALL: [Part; 3] = [Part::Surface, Part::NavPane, Part::JumpList];

    /// Key in `state.json` and in the status.
    pub fn key(self) -> &'static str {
        match self {
            Part::Surface => "surface",
            Part::NavPane => "nav_pane",
            Part::JumpList => "jump_list",
        }
    }

    /// Spelling of `--part` and `se desktop part`.
    pub fn cli_name(self) -> &'static str {
        match self {
            Part::Surface => "surface",
            Part::NavPane => "nav-pane",
            Part::JumpList => "jump-list",
        }
    }

    pub fn parse_cli(value: &str) -> Option<Part> {
        Part::ALL.into_iter().find(|part| part.cli_name() == value)
    }

    /// Whether the part exists on `platform` at all (Explorer parts only on
    /// Windows); a part can still be unavailable in the running session.
    pub fn exists_on(self, platform: Platform) -> bool {
        self == Part::Surface || platform == Platform::Windows
    }
}

/// `parts` of `state.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartSet {
    pub surface: bool,
    pub nav_pane: bool,
    pub jump_list: bool,
}

impl PartSet {
    pub fn of(parts: &[Part]) -> PartSet {
        let mut set = PartSet::default();
        for part in parts {
            set.set(*part, true);
        }
        set
    }

    pub fn contains(self, part: Part) -> bool {
        match part {
            Part::Surface => self.surface,
            Part::NavPane => self.nav_pane,
            Part::JumpList => self.jump_list,
        }
    }

    pub fn set(&mut self, part: Part, on: bool) {
        match part {
            Part::Surface => self.surface = on,
            Part::NavPane => self.nav_pane = on,
            Part::JumpList => self.jump_list = on,
        }
    }

    pub fn any(self) -> bool {
        self.surface || self.nav_pane || self.jump_list
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HaltReason {
    CrashLoop,
}

/// The shell stopped after three crashes within ten minutes (C5.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Halt {
    pub reason: HaltReason,
    /// se version that crashed; another version resumes (Ä30).
    pub version: String,
    pub at: i64,
}

/// Content of `state.json` (C5.4); there is no field `mode` (Ä1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleState {
    pub schema: u32,
    pub state: State,
    pub parts: PartSet,
    pub changed_at: i64,
    pub halted: Option<Halt>,
    pub ledger: Vec<LedgerEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    /// The module is `absent`; `se desktop enable` adds it.
    NotAdded,
    /// The part does not exist here or the session does not offer it.
    PartUnavailable(Part),
    Schema(u32),
    /// `state.json` names `absent`, which is never stored.
    StoredAbsent,
    Json(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::NotAdded => write!(
                formatter,
                "se Desktop ist nicht hinzugefügt; zuerst `se desktop enable`"
            ),
            StateError::PartUnavailable(part) => write!(
                formatter,
                "Teil {} ist hier nicht verfügbar",
                part.cli_name()
            ),
            StateError::Schema(schema) => {
                write!(formatter, "state.json hat das unbekannte Schema {schema}")
            }
            StateError::StoredAbsent => write!(formatter, "state.json enthält den Zustand absent"),
            StateError::Json(error) => write!(formatter, "state.json ist ungültig: {error}"),
        }
    }
}

impl std::error::Error for StateError {}

impl ModuleState {
    pub fn from_json(text: &str) -> Result<ModuleState, StateError> {
        let module: ModuleState =
            serde_json::from_str(text).map_err(|error| StateError::Json(error.to_string()))?;
        if module.schema != STATE_SCHEMA {
            return Err(StateError::Schema(module.schema));
        }
        if module.state == State::Absent {
            return Err(StateError::StoredAbsent);
        }
        Ok(module)
    }

    pub fn to_json(&self) -> Result<String, StateError> {
        serde_json::to_string_pretty(self).map_err(|error| StateError::Json(error.to_string()))
    }

    /// C5.6: a normal GUI start starts the shell when the module is enabled
    /// with a part on and not halted for the running version (Ä30).
    pub fn should_resume(&self, running_version: &str) -> bool {
        let halted_here = matches!(&self.halted, Some(halt) if halt.version == running_version);
        self.state == State::Enabled && self.parts.any() && !halted_here
    }

    pub fn halt(&mut self, version: &str, at: i64) {
        self.halted = Some(Halt {
            reason: HaltReason::CrashLoop,
            version: version.to_string(),
            at,
        });
    }

    /// `se desktop restart` and the button "Neu starten" clear the halt.
    pub fn clear_halt(&mut self) -> bool {
        self.halted.take().is_some()
    }

    /// Records an applied registration; the same kind and target once.
    pub fn record(&mut self, entry: LedgerEntry) {
        self.forget(entry.kind, &entry.target);
        self.ledger.push(entry);
    }

    pub fn forget(&mut self, kind: Registration, target: &str) {
        self.ledger
            .retain(|known| !(known.kind == kind && known.target == target));
    }
}

/// `se desktop enable [--part …]`. From `absent`: the requested parts, else
/// every available part. From `disabled`: the stored parts, plus requested
/// ones; with no part stored, every available part. A requested part that is
/// not available is refused. Entering `enabled` clears a crash-loop halt.
pub fn enable(
    current: Option<&ModuleState>,
    requested: Option<&[Part]>,
    available: PartSet,
    now: i64,
) -> Result<ModuleState, StateError> {
    if let Some(part) = requested
        .unwrap_or_default()
        .iter()
        .copied()
        .find(|part| !available.contains(*part))
    {
        return Err(StateError::PartUnavailable(part));
    }
    let Some(current) = current else {
        return Ok(ModuleState {
            schema: STATE_SCHEMA,
            state: State::Enabled,
            parts: requested.map_or(available, PartSet::of),
            changed_at: now,
            halted: None,
            ledger: Vec::new(),
        });
    };
    let mut next = current.clone();
    match requested {
        Some(parts) => {
            for part in parts {
                next.parts.set(*part, true);
            }
        }
        None if !next.parts.any() => next.parts = available,
        None => {}
    }
    if next.state != State::Enabled {
        next.state = State::Enabled;
        next.halted = None;
    }
    if next != *current {
        next.changed_at = now;
    }
    Ok(next)
}

/// `se desktop disable`: keeps parts, configuration, arrangement and ledger;
/// the os layer removes the traces of [`super::ledger::removal_plan`].
pub fn disable(current: Option<&ModuleState>, now: i64) -> Option<ModuleState> {
    let mut next = current?.clone();
    if next.state != State::Disabled {
        next.state = State::Disabled;
        next.changed_at = now;
    }
    Some(next)
}

/// `se desktop part <TEIL> on|off`, in `enabled` and `disabled`.
pub fn set_part(
    current: Option<&ModuleState>,
    part: Part,
    on: bool,
    available: PartSet,
    now: i64,
) -> Result<ModuleState, StateError> {
    let current = current.ok_or(StateError::NotAdded)?;
    if on && !available.contains(part) {
        return Err(StateError::PartUnavailable(part));
    }
    let mut next = current.clone();
    next.parts.set(part, on);
    if next != *current {
        next.changed_at = now;
    }
    Ok(next)
}
