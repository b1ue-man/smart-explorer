use std::path::{is_separator, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::access_grant::{
    valid_account_name, AccessGrant, AccessGrantChange, AccountProfile, Capability,
    CapabilityDecision, REMOTE_ACCOUNT_NAME,
};
use super::access_grant_wire::{self, AccessRecord, KeptParts};
use super::direct_protocol::DirectRequestId;
use super::exec_types::{ExecCommand, ExecStart};
use super::profiles::ShareProfiles;

#[path = "exec_program_path.rs"]
pub(crate) mod program_path;
#[path = "exec_policy_wire.rs"]
mod wire;

use self::program_path::ProgramSearch;

/// Longest allowed program path (`PATH_MAX`).
const MAX_PROGRAM_BYTES: usize = 4096;
/// Longest locale or terminal name a program-list job takes over.
const MAX_ENV_VALUE_BYTES: usize = 64;

/// The OS account a granted device's commands run as on this host.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecRunAs {
    /// The account of the Smart Explorer worker ("Standardnutzer"). Every
    /// grant made before run-as existed is ported to it, with unchanged rights.
    #[default]
    Standard,
    /// A named local account; only Linux hosts whose worker runs as root can
    /// switch to it.
    Account { name: String },
}

impl ExecRunAs {
    pub fn account_name(&self) -> Option<&str> {
        match self {
            Self::Standard => None,
            Self::Account { name } => Some(name),
        }
    }
}

/// Requested run-as account and program list of one grant change.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecGrantSettings {
    #[serde(default)]
    pub run_as: ExecRunAs,
    /// Empty: the full shell as before. Otherwise only argv starts of exactly
    /// these programs (bare names or absolute paths, found in the host's
    /// fixed search path); no shell command and no terminal.
    #[serde(default)]
    pub programs: Vec<String>,
}

impl ExecGrantSettings {
    pub fn validate(&self) -> Result<(), String> {
        if let ExecRunAs::Account { name } = &self.run_as {
            if !valid_account_name(name) {
                return Err(format!("invalid local account name {name:?}"));
            }
        }
        for program in &self.programs {
            if program.trim().is_empty()
                || program.len() > MAX_PROGRAM_BYTES
                || program.contains('\0')
            {
                return Err(format!("invalid allowed program {program:?}"));
            }
            // A relative path would depend on the working directory of
            // whoever starts it; only a bare name (looked up in the host's
            // fixed search path) or an absolute path names one program.
            if program.chars().any(is_separator) && !Path::new(program).is_absolute() {
                return Err(format!(
                    "allowed program {program:?} must be a bare name or an absolute path"
                ));
            }
        }
        Ok(())
    }
}

impl ExecGrantSettings {
    /// `validate`, plus: another account needs a host that can run commands
    /// as one (a Linux worker running as root). Windows, Android and a
    /// Linux worker without root refuse it with the reason, and the grant
    /// stays as it is.
    pub fn validate_for_this_host(&self) -> Result<(), String> {
        self.validate()?;
        if self.run_as.account_name().is_some() && !super::exec_platform::run_as_supported() {
            return Err(
                "this host runs commands only as the Smart Explorer user (Standardnutzer); \
                 another account needs a Linux worker running as root"
                    .into(),
            );
        }
        Ok(())
    }
}

/// Local access right (commands, terminal and, through S4, the screen) for
/// one exact pinned peer identity.
///
/// File access never implies execution access. New and migrated policies are
/// disabled, and identity or base-authorization changes advance the local
/// revision while returning the policy to deny. `run_as` and `programs`
/// narrow an enabled grant; profiles written before they existed read as the
/// Standardnutzer with the full shell, exactly the rights they had.
///
/// The access right itself is `access_grant()`: the `access` object when the
/// grant needs one (C1), else the fields above ported as before. `enabled`
/// and `run_as` mirror it for every surface that predates it: `enabled` is
/// true when an Exec capability is not off.
///
/// Stored through `wire::ExecGrantWire`: a grant that older versions cannot
/// express is stored as switched off for them.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "wire::ExecGrantWire", into = "wire::ExecGrantWire")]
pub struct ExecGrant {
    pub enabled: bool,
    pub policy_revision: u64,
    pub changed_at: i64,
    pub source_request_id: Option<DirectRequestId>,
    pub source_decision_revision: Option<u64>,
    pub run_as: ExecRunAs,
    pub programs: Vec<String>,
    /// The stored `access` object; `None` when the fields above express the
    /// grant exactly.
    pub(crate) access: Option<AccessRecord>,
}

impl ExecGrant {
    /// The access right of this grant, without its end applied.
    pub(crate) fn access_grant(&self) -> AccessGrant {
        match &self.access {
            None => access_grant_wire::ported(self.enabled, &self.run_as, &self.programs),
            Some(AccessRecord::Known { grant, .. }) => {
                let mut grant = grant.clone();
                if !self.enabled {
                    // `enabled` cleared on its own (a journal mask) keeps
                    // every Exec capability off.
                    for capability in Capability::EXEC {
                        grant.set(capability, CapabilityDecision::Off);
                    }
                }
                grant
            }
            Some(AccessRecord::Unknown(_)) => {
                AccessGrant::new(AccountProfile::from_run_as(&self.run_as))
            }
        }
    }

    /// The decision for `capability` at `now` (Unix seconds, host clock).
    pub(crate) fn decision_at(&self, capability: Capability, now: i64) -> CapabilityDecision {
        self.access_grant().decision_at(capability, now)
    }

    /// Unix seconds from which every capability of the grant is off.
    pub(crate) fn valid_until(&self) -> Option<i64> {
        self.access_grant().valid_until
    }

    /// Whether any capability, Exec or screen, is on (the end not applied).
    pub(crate) fn grants_any(&self) -> bool {
        self.access_grant().grants_any()
    }

    /// The capabilities that `self` narrows against `old` (C5); empty for a
    /// pure widening or no change.
    pub(crate) fn narrowed_against(&self, old: &ExecGrant) -> Vec<Capability> {
        let programs_narrowed =
            super::access_revocation::program_list_narrows(&old.programs, &self.programs);
        self.access_grant()
            .narrowing_against(&old.access_grant(), programs_narrowed)
    }

    /// Applies `change` in the next revision; a refused change leaves the
    /// grant as it was.
    pub(crate) fn apply_access_change(
        &mut self,
        change: &AccessGrantChange,
        changed_at: i64,
    ) -> Result<(), String> {
        let mut grant = self.access_grant();
        let mut programs = self.programs.clone();
        grant.apply(change, &mut programs)?;
        let kept = self.kept_parts();
        self.advance(changed_at).map_err(str::to_string)?;
        self.store(grant, programs, kept);
        Ok(())
    }

    /// Enables or disables and, when `settings` is given, replaces run-as
    /// account and program list in the same revision step. Enabling allows
    /// commands and, without a program list, the terminal; disabling turns
    /// both off. The rest of the access right stays (D.2.3).
    pub(crate) fn set_runtime_policy(
        &mut self,
        enabled: bool,
        settings: Option<&ExecGrantSettings>,
        changed_at: i64,
    ) -> Result<(), &'static str> {
        let mut grant = self.access_grant();
        let mut programs = self.programs.clone();
        if let Some(settings) = settings {
            let account = AccountProfile::from_run_as(&settings.run_as);
            // Surfaces that predate the remote account repeat it by name.
            let repeats_remote = grant.account == AccountProfile::Remote
                && account.account_name() == Some(REMOTE_ACCOUNT_NAME);
            if !repeats_remote {
                grant.account = account;
            }
            programs = settings.programs.clone();
        }
        let (commands, terminal) = match (enabled, programs.is_empty()) {
            (false, _) => (CapabilityDecision::Off, CapabilityDecision::Off),
            (true, true) => (CapabilityDecision::Allow, CapabilityDecision::Allow),
            (true, false) => (CapabilityDecision::Allow, CapabilityDecision::Off),
        };
        grant.set(Capability::ExecCommands, commands);
        grant.set(Capability::ExecTerminal, terminal);
        let kept = self.kept_parts();
        self.advance(changed_at)?;
        self.store(grant, programs, kept);
        Ok(())
    }

    /// Every capability off without a new revision: what a pending journal
    /// entry leaves of the grant until it is applied (fail closed).
    pub(crate) fn mask_off(&mut self) {
        self.enabled = false;
        if let Some(AccessRecord::Known { grant, .. }) = &mut self.access {
            for capability in Capability::ALL {
                grant.set(capability, CapabilityDecision::Off);
            }
        }
    }

    fn advance(&mut self, changed_at: i64) -> Result<(), &'static str> {
        self.policy_revision = self
            .policy_revision
            .checked_add(1)
            .ok_or("Exec policy revision exhausted")?;
        self.changed_at = changed_at;
        self.source_request_id = None;
        self.source_decision_revision = None;
        Ok(())
    }

    fn kept_parts(&self) -> KeptParts {
        match &self.access {
            Some(AccessRecord::Known { kept, .. }) => kept.clone(),
            _ => KeptParts::default(),
        }
    }

    /// Keeps `grant` in the fields of readers up to 0.5.182 when they express
    /// it exactly, else in the `access` object with mirrored fields.
    fn store(&mut self, grant: AccessGrant, programs: Vec<String>, kept: KeptParts) {
        self.programs = programs;
        match access_grant_wire::legacy_form(&grant, &self.programs) {
            Some((enabled, run_as)) if kept.is_empty() => {
                self.enabled = enabled;
                self.run_as = run_as;
                self.access = None;
            }
            _ => {
                self.enabled = grant.grants_exec();
                self.run_as = grant.account.run_as();
                self.access = Some(AccessRecord::Known { grant, kept });
            }
        }
    }

    /// After reading a stored `access` object or turning every capability
    /// off: the mirrored fields, or the old fields alone when they express
    /// the grant exactly. An unknown object stays as it is, switched off.
    fn settle_stored_access(&mut self) {
        match self.access.take() {
            Some(AccessRecord::Known { grant, kept }) => {
                let programs = std::mem::take(&mut self.programs);
                self.store(grant, programs, kept);
            }
            Some(unknown) => {
                self.enabled = false;
                self.access = Some(unknown);
            }
            None => {}
        }
    }

    /// True when this grant is narrowed: a program list or another account.
    pub(crate) fn is_restricted(&self) -> bool {
        Self::restricts(&self.run_as, &self.programs)
    }

    fn restricts(run_as: &ExecRunAs, programs: &[String]) -> bool {
        !programs.is_empty() || *run_as != ExecRunAs::Standard
    }

    /// Applies the grant to `start` before any process exists. A grant
    /// without a program list is the full shell: it refuses nothing and
    /// returns `Ok(None)`. A grant with a program list refuses shell
    /// commands, terminals, every caller environment variable but locale and
    /// `TERM`, and every program that does not resolve (as an absolute path
    /// or in the host's fixed search path, never through the caller's `PATH`)
    /// to a listed one; it returns the resolved path the job starts with.
    pub(crate) fn authorize_start(
        &self,
        start: &ExecStart,
        search: &ProgramSearch,
    ) -> Result<Option<PathBuf>, String> {
        if self.programs.is_empty() {
            return Ok(None);
        }
        let program = match &start.command {
            ExecCommand::Argv { program, .. } => program,
            ExecCommand::Shell { .. } => {
                return Err(
                    "this host allows this device only its listed programs, no shell commands"
                        .into(),
                );
            }
            ExecCommand::Login => {
                return Err(
                    "this host allows this device only its listed programs, no terminal".into(),
                );
            }
        };
        for (name, value) in &start.env {
            if !permitted_name(name) {
                return Err(format!(
                    "this host allows this device only its listed programs; the environment \
                     variable {name:?} is not allowed (only LANG, LC_* and TERM)"
                ));
            }
            if !plain_value(value) {
                return Err(format!(
                    "this host allows this device only its listed programs; the value of \
                     {name} must be a plain locale or terminal name"
                ));
            }
        }
        program_path::authorize(program, &self.programs, search).map(Some)
    }

    /// Enables or disables in the next revision; account and list stay.
    pub(crate) fn set_runtime_enabled(
        &mut self,
        enabled: bool,
        changed_at: i64,
    ) -> Result<(), &'static str> {
        self.set_runtime_policy(enabled, None, changed_at)
    }

    pub(crate) fn reset_for_identity_change(&mut self, changed_at: i64) {
        self.disable(changed_at, None, None);
    }

    pub(crate) fn disable_for_base_decision(
        &mut self,
        changed_at: i64,
        request_id: DirectRequestId,
        decision_revision: u64,
    ) {
        self.disable(changed_at, Some(request_id), Some(decision_revision));
    }

    pub(crate) fn disable_without_decision(&mut self, changed_at: i64) {
        self.disable(changed_at, None, None);
    }

    /// Every capability, Exec and screen, off in the next revision; account,
    /// list and end stay.
    fn disable(
        &mut self,
        changed_at: i64,
        source_request_id: Option<DirectRequestId>,
        source_decision_revision: Option<u64>,
    ) {
        self.mask_off();
        self.policy_revision = self.policy_revision.saturating_add(1);
        self.changed_at = changed_at;
        self.source_request_id = source_request_id;
        self.source_decision_revision = source_decision_revision;
        self.settle_stored_access();
    }
}

/// The only caller variables a job with a program list takes over: locale
/// and terminal type, as `sshd` does with `AcceptEnv LANG LC_*`.
fn permitted_name(name: &str) -> bool {
    name == "LANG"
        || name == "TERM"
        || name.strip_prefix("LC_").is_some_and(|rest| {
            !rest.is_empty()
                && rest.len() <= 32
                && rest
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

/// A plain name (`en_US.UTF-8`, `xterm-256color`): no path, no separator.
fn plain_value(value: &str) -> bool {
    value.len() <= MAX_ENV_VALUE_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._@+-".contains(&byte))
}

/// Legacy profiles used one export-wide Exec bit. It was never a safe grant,
/// so migration must ignore it and deny every exact peer independently.
pub(super) fn reset_all_for_legacy_migration(profiles: &mut ShareProfiles) {
    for grant in &mut profiles.direct_grants {
        grant.exec = ExecGrant::default();
    }
    for room in &mut profiles.rooms {
        for member in &mut room.members {
            member.exec = ExecGrant::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExecGrant;
    use crate::share::DirectRequestId;

    #[test]
    fn policy_is_default_deny_and_disable_advances_revision() {
        let mut policy = ExecGrant::default();
        assert!(!policy.enabled);
        assert_eq!(policy.policy_revision, 0);

        policy.enabled = true;
        policy.disable_without_decision(17);
        assert!(!policy.enabled);
        assert_eq!(policy.policy_revision, 1);
        assert_eq!(policy.changed_at, 17);
        assert!(policy.source_request_id.is_none());
    }

    #[test]
    fn base_decision_source_is_recorded_when_disabling() {
        let request_id = DirectRequestId::parse("01234567-89ab-4def-8123-456789abcdef").unwrap();
        let mut policy = ExecGrant {
            enabled: true,
            ..ExecGrant::default()
        };
        policy.disable_for_base_decision(29, request_id.clone(), 4);
        assert!(!policy.enabled);
        assert_eq!(policy.source_request_id, Some(request_id));
        assert_eq!(policy.source_decision_revision, Some(4));
    }

    #[test]
    fn runtime_change_is_monotonic_and_clears_base_decision_source() {
        let request_id = DirectRequestId::parse("01234567-89ab-4def-8123-456789abcdef").unwrap();
        let mut policy = ExecGrant {
            source_request_id: Some(request_id),
            source_decision_revision: Some(3),
            ..ExecGrant::default()
        };
        policy.set_runtime_enabled(true, 41).unwrap();
        assert!(policy.enabled);
        assert_eq!(policy.policy_revision, 1);
        assert_eq!(policy.changed_at, 41);
        assert!(policy.source_request_id.is_none());
        assert!(policy.source_decision_revision.is_none());
    }
}

#[cfg(test)]
#[path = "exec_policy_tests.rs"]
mod luecken_task_tests;
