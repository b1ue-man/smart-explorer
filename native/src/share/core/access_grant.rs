//! Access right of one paired device or room member on this host (strand S1,
//! decision C1, contract V7): the account its commands run as, one decision
//! per capability and an optional end. The screen strand (S4) uses
//! `Capability`, `CapabilityDecision` and `CapabilityState` as they are. The
//! program list stays the field `ExecGrant::programs`; the stored form is in
//! `access_grant_wire.rs`.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::share::access_constraints::{AccessConstraints, AccessRequest};
pub use crate::share::access_relation_view::AccessGrantView;
use crate::share::access_steering as steering;
use crate::share::exec_policy::{ExecGrantSettings, ExecRunAs};

/// The restricted remote-access account of `AccountProfile::Remote`.
pub const REMOTE_ACCOUNT_NAME: &str = "se-remote";
/// Longest local account name (`LOGIN_NAME_MAX` is 256 on Linux).
const MAX_ACCOUNT_NAME_BYTES: usize = 256;

/// What a grant can allow. The set is fixed in this enum; there is no
/// registration of capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Commands: argv and shell starts.
    ExecCommands,
    /// The interactive terminal: the login start with a terminal.
    ExecTerminal,
    /// Watching the screen (S4).
    ScreenView,
    /// Controlling the screen (S4).
    ScreenControl,
    /// Launch one host-resolved application in an admitted session.
    AppLaunch,
    /// Explicit machine-authorized login/session access.
    LoginAccess,
    /// One explicitly bounded TCP forwarding direction and target.
    TcpForward,
}

impl Capability {
    pub const ALL: [Capability; 7] = [
        Self::ExecCommands,
        Self::ExecTerminal,
        Self::ScreenView,
        Self::ScreenControl,
        Self::AppLaunch,
        Self::LoginAccess,
        Self::TcpForward,
    ];
    pub const EXEC: [Capability; 2] = [Self::ExecCommands, Self::ExecTerminal];

    /// Key in the stored form, in JSON and in `APPROVAL_PENDING`.
    pub fn key(self) -> &'static str {
        match self {
            Self::ExecCommands => "exec_commands",
            Self::ExecTerminal => "exec_terminal",
            Self::ScreenView => "screen_view",
            Self::ScreenControl => "screen_control",
            Self::AppLaunch => "app_launch",
            Self::LoginAccess => "login_access",
            Self::TcpForward => "tcp_forward",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.key() == key)
    }

    /// The account profile applies to the Exec capabilities only (E1).
    pub fn is_exec(self) -> bool {
        matches!(self, Self::ExecCommands | Self::ExecTerminal)
    }

    pub fn requires_context(self) -> bool {
        matches!(self, Self::AppLaunch | Self::LoginAccess | Self::TcpForward)
    }

    pub fn uses_account(self) -> bool {
        self.is_exec() || matches!(self, Self::AppLaunch | Self::LoginAccess)
    }

    fn index(self) -> usize {
        match self {
            Self::ExecCommands => 0,
            Self::ExecTerminal => 1,
            Self::ScreenView => 2,
            Self::ScreenControl => 3,
            Self::AppLaunch => 4,
            Self::LoginAccess => 5,
            Self::TcpForward => 6,
        }
    }
}

/// Decision of the host user for one capability. Missing means `Off`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    #[default]
    Off,
    /// Every start or session asks the host user first.
    Ask,
    Allow,
}

impl CapabilityDecision {
    /// Rank of a decision: `off` < `ask` < `allow`. A lower rank narrows.
    pub fn rank(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Ask => 1,
            Self::Allow => 2,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ask => "ask",
            Self::Allow => "allow",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Off, Self::Ask, Self::Allow]
            .into_iter()
            .find(|decision| decision.key() == key)
    }

    pub fn is_on(self) -> bool {
        self != Self::Off
    }
}

/// One capability of one grant at one moment (V7): the decision, and the
/// revision and change time of the whole grant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapabilityState {
    pub decision: CapabilityDecision,
    pub policy_revision: u64,
    pub changed_at: i64,
}

/// The OS account commands and terminals of a grant run as.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountProfile {
    /// The account of the Smart Explorer worker ("Mein Konto").
    #[default]
    Own,
    /// The restricted remote-access account `se-remote`.
    Remote,
    /// An existing local account, by name.
    Account { name: String },
}

impl AccountProfile {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::Remote => "remote",
            Self::Account { .. } => "account",
        }
    }

    /// The run-as account of the launch path and of readers up to 0.5.182.
    pub fn run_as(&self) -> ExecRunAs {
        match self.account_name() {
            None => ExecRunAs::Standard,
            Some(name) => ExecRunAs::Account { name: name.into() },
        }
    }

    pub fn from_run_as(run_as: &ExecRunAs) -> Self {
        match run_as.account_name() {
            None => Self::Own,
            Some(name) => Self::Account { name: name.into() },
        }
    }

    /// The local account another than the worker's; `None` for `Own`.
    pub fn account_name(&self) -> Option<&str> {
        match self {
            Self::Own => None,
            Self::Remote => Some(REMOTE_ACCOUNT_NAME),
            Self::Account { name } => Some(name),
        }
    }

    /// How surfaces name the account; `own_label` is the worker's account.
    pub fn label(&self, own_label: &str) -> String {
        self.account_name().unwrap_or(own_label).to_string()
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Account { name } if !valid_account_name(name) => {
                Err(format!("invalid local account name {name:?}"))
            }
            _ => Ok(()),
        }
    }
}

/// A local account name a grant may name: 1 to 256 bytes of `A-Za-z0-9._-$`,
/// not starting with `-`.
pub(crate) fn valid_account_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ACCOUNT_NAME_BYTES
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-$".contains(&byte))
}

/// End of a grant in a change: `"none"` removes it, `{"at": SECONDS}` sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expiry {
    None,
    At(i64),
}

/// The access right of one grant without its program list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessGrant {
    pub account: AccountProfile,
    decisions: [CapabilityDecision; 7],
    /// V2 remains V2 after revocation; a downgrade never restores an old grant.
    pub(crate) version: u64,
    pub constraints: AccessConstraints,
    /// Unix seconds of the host clock from which every capability is off.
    pub valid_until: Option<i64>,
}

impl Default for AccessGrant {
    fn default() -> Self {
        Self {
            account: AccountProfile::default(),
            decisions: [CapabilityDecision::Off; 7],
            version: 1,
            constraints: AccessConstraints::default(),
            valid_until: None,
        }
    }
}

impl AccessGrant {
    pub fn new(account: AccountProfile) -> Self {
        Self {
            account,
            ..Self::default()
        }
    }

    pub fn with(mut self, capability: Capability, decision: CapabilityDecision) -> Self {
        self.set(capability, decision);
        self
    }

    pub fn decision(&self, capability: Capability) -> CapabilityDecision {
        self.decisions[capability.index()]
    }

    pub fn set(&mut self, capability: Capability, decision: CapabilityDecision) {
        self.decisions[capability.index()] = decision;
        if capability.requires_context() && decision.is_on() {
            self.version = 2;
        }
    }

    pub fn capabilities(&self) -> impl Iterator<Item = (Capability, CapabilityDecision)> + '_ {
        Capability::ALL
            .into_iter()
            .map(|capability| (capability, self.decision(capability)))
    }

    /// Whether the end has come at `now` (inclusive).
    pub fn expired_at(&self, now: i64) -> bool {
        self.valid_until.is_some_and(|end| now >= end)
    }

    /// The decision that applies at `now`: off once the grant has ended.
    pub fn decision_at(&self, capability: Capability, now: i64) -> CapabilityDecision {
        if self.expired_at(now) || capability.requires_context() {
            CapabilityDecision::Off
        } else if steering::includes(capability) && steering::steers(self, now) {
            CapabilityDecision::Allow
        } else {
            self.decision(capability)
        }
    }

    /// A contextual capability never allows through `decision_at` alone.
    pub fn decision_for(&self, request: &AccessRequest, now: i64) -> CapabilityDecision {
        if steering::steers(self, now) && steering::includes_request(request) {
            return CapabilityDecision::Allow;
        }
        if self.expired_at(now)
            || self.constraints.validate().is_err()
            || !self.constraints.allows(request, now)
        {
            CapabilityDecision::Off
        } else {
            self.decision(request.capability())
        }
    }

    pub fn version(&self) -> u64 {
        if self.version == 2 || !self.constraints.is_empty() {
            2
        } else {
            1
        }
    }

    pub fn grants_any(&self) -> bool {
        self.decisions.iter().any(|decision| decision.is_on())
    }

    pub fn grants_exec(&self) -> bool {
        Capability::EXEC
            .into_iter()
            .any(|capability| self.decision(capability).is_on())
    }

    /// The capabilities that going from `old` to `self` narrows (C5): their
    /// rank sinks, the account changes (Exec capabilities), the program list
    /// changes (commands), or the end comes earlier or is new. Only a
    /// capability that was on can narrow. An empty result is a pure widening.
    pub fn narrowing_against(
        &self,
        old: &AccessGrant,
        program_list_changed: bool,
    ) -> Vec<Capability> {
        let earlier_end = match (old.valid_until, self.valid_until) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(before), Some(after)) => after < before,
        };
        let account_changed = self.account != old.account;
        Capability::ALL
            .into_iter()
            .filter(|&capability| {
                let before = old.decision(capability);
                before.is_on()
                    && (self.decision(capability).rank() < before.rank()
                        || earlier_end
                        || (capability.uses_account() && account_changed)
                        || (capability == Capability::ExecCommands && program_list_changed)
                        || self.constraints.narrows(&old.constraints, capability))
            })
            .collect()
    }

    /// Applies `change`; `programs` is the grant's program list. A program
    /// list allows no terminal: a change that sets a list turns the terminal
    /// off unless the same change asks for a terminal, which is refused.
    pub fn apply(
        &mut self,
        change: &AccessGrantChange,
        programs: &mut Vec<String>,
    ) -> Result<(), String> {
        let mut next = self.clone();
        let mut next_programs = programs.clone();
        if let Some(account) = &change.account {
            next.account = account.clone();
        }
        for (capability, decision) in &change.capabilities {
            next.set(*capability, *decision);
        }
        if let Some(constraints) = &change.constraints {
            next.constraints = constraints.clone();
            next.version = 2;
        }
        if let Some(list) = &change.programs {
            next_programs = list.clone();
        }
        match change.expiry {
            Some(Expiry::None) => next.valid_until = None,
            Some(Expiry::At(end)) => next.valid_until = Some(end),
            None => {}
        }
        if !next_programs.is_empty()
            && next.decision(Capability::ExecTerminal).is_on()
            && !change.capabilities.contains_key(&Capability::ExecTerminal)
        {
            next.set(Capability::ExecTerminal, CapabilityDecision::Off);
        }
        next.validate(&next_programs)?;
        *self = next;
        *programs = next_programs;
        Ok(())
    }

    /// The rules every stored grant keeps.
    pub fn validate(&self, programs: &[String]) -> Result<(), String> {
        self.account.validate()?;
        self.constraints.validate()?;
        for capability in Capability::ALL {
            if capability.requires_context()
                && self.decision(capability).is_on()
                && !self.constraints.has(capability)
            {
                return Err(format!(
                    "{} requires explicit target constraints",
                    capability.key()
                ));
            }
        }
        ExecGrantSettings {
            run_as: ExecRunAs::Standard,
            programs: programs.to_vec(),
        }
        .validate()?;
        if !programs.is_empty() && self.decision(Capability::ExecTerminal).is_on() {
            return Err("a grant with a program list allows no terminal".into());
        }
        if self.valid_until.is_some_and(|end| end <= 0) {
            return Err("the end of a grant must be a time after 1970".into());
        }
        Ok(())
    }
}

/// One change of an access grant (D.2.3). Every field is optional; a missing
/// field keeps its value, and only the named capabilities change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGrantChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountProfile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<Capability, CapabilityDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<Expiry>,
    /// Complete replacement; an omitted field preserves every condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<AccessConstraints>,
}

impl AccessGrantChange {
    pub fn is_empty(&self) -> bool {
        self.account.is_none()
            && self.capabilities.is_empty()
            && self.programs.is_none()
            && self.expiry.is_none()
            && self.constraints.is_none()
    }
}

/// The presets of the access dialog (C1). They are changes, not stored
/// templates; `ask` turns every capability they allow into `ask`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessPreset {
    /// "Volle Rechte als mein Konto": commands and terminal as the worker.
    FullOwn,
    /// "Eingeschränktes Fernzugriffskonto": commands and terminal as `se-remote`.
    RemoteAccount,
    /// "Nur bestimmte Programme": commands of the listed programs only.
    Programs,
}

impl AccessPreset {
    pub fn change(self, programs: Vec<String>, ask: bool) -> AccessGrantChange {
        let on = if ask {
            CapabilityDecision::Ask
        } else {
            CapabilityDecision::Allow
        };
        let (account, terminal, programs) = match self {
            Self::FullOwn => (AccountProfile::Own, on, Vec::new()),
            Self::RemoteAccount => (AccountProfile::Remote, on, Vec::new()),
            Self::Programs => (AccountProfile::Own, CapabilityDecision::Off, programs),
        };
        AccessGrantChange {
            account: Some(account),
            capabilities: BTreeMap::from([
                (Capability::ExecCommands, on),
                (Capability::ExecTerminal, terminal),
            ]),
            programs: Some(programs),
            expiry: None,
            constraints: None,
        }
    }
}

#[cfg(test)]
#[path = "access_grant_tests.rs"]
mod tests;
