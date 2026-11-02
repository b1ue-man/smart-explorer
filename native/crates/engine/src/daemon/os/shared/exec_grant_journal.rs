use std::io::{self, Read};

use serde::{Deserialize, Serialize};

use super::{default_home, stop_service_locked, ShareHost, ShareHostState};

#[path = "exec_grant_journal_storage.rs"]
mod storage;

#[path = "exec_grant_journal_rights.rs"]
mod rights;

#[path = "exec_grant_journal_follow_ups.rs"]
mod follow_ups;

use storage::{clear_entry, decode_entry, write_entry};

const JOURNAL_VERSION: u32 = 2;
/// An entry of „Bestätigen“ (W8, review 2026-10-31): several grants and
/// the rest of the connection rights. Daemons that know only version 2
/// refuse it and fail closed (every grant masked, worker stopped).
const CONFIRM_JOURNAL_VERSION: u32 = 3;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JournalPhase {
    PendingApply,
    PendingDeny,
}

impl JournalPhase {
    fn retry_state(self) -> ExecGrantRetryState {
        match self {
            Self::PendingApply => ExecGrantRetryState::PendingApply,
            Self::PendingDeny => ExecGrantRetryState::PendingDeny,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct JournalEntry {
    version: u32,
    operation_id: String,
    phase: JournalPhase,
    expected_policy_revision: u64,
    mutation: crate::share::ExecGrantMutation,
    /// Version 3 only: the further grants of the same confirmation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    more: Vec<rights::GrantStep>,
    /// Version 3 only: the confirmation itself, persisted in the same
    /// profile transaction as the grants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rights: Option<rights::RightsStep>,
}

impl JournalEntry {
    fn new(
        mutation: crate::share::ExecGrantMutation,
        expected_policy_revision: u64,
    ) -> Result<Self, String> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|error| format!("Exec-Grant operation id: {error}"))?;
        let operation_id = random.iter().map(|byte| format!("{byte:02x}")).collect();
        // A grant that allows anything (Exec or screen) is persisted first.
        let phase = if mutation.policy.grants_any() {
            JournalPhase::PendingApply
        } else {
            JournalPhase::PendingDeny
        };
        let entry = Self {
            version: JOURNAL_VERSION,
            operation_id,
            phase,
            expected_policy_revision,
            mutation,
            more: Vec::new(),
            rights: None,
        };
        entry.validate()?;
        Ok(entry)
    }

    /// Every grant of the entry with the revision it replaces.
    fn steps(&self) -> impl Iterator<Item = (&crate::share::ExecGrantMutation, u64)> {
        std::iter::once((&self.mutation, self.expected_policy_revision)).chain(
            self.more
                .iter()
                .map(|step| (&step.mutation, step.expected_policy_revision)),
        )
    }

    fn validate(&self) -> Result<(), String> {
        let expected_version = if self.rights.is_some() {
            CONFIRM_JOURNAL_VERSION
        } else {
            JOURNAL_VERSION
        };
        if self.version != expected_version
            || (self.version == JOURNAL_VERSION && !self.more.is_empty())
        {
            return Err(format!(
                "unsupported Exec-Grant journal version {}",
                self.version
            ));
        }
        if self.operation_id.len() != 32
            || !self
                .operation_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid Exec-Grant journal operation id".into());
        }
        // A confirmation that grants anything anywhere is persisted first;
        // its revocations reach the worker before that (`execute_locked`).
        let phase_enabled = self.phase == JournalPhase::PendingApply;
        if self
            .steps()
            .any(|(mutation, _)| mutation.policy.grants_any())
            != phase_enabled
        {
            return Err("Exec-Grant journal phase contradicts desired policy".into());
        }
        let mut targets = Vec::new();
        for (mutation, expected) in self.steps() {
            mutation.validate_persisted_shape(expected)?;
            if targets.contains(&&mutation.target) {
                return Err("Exec-Grant journal names one grant twice".into());
            }
            targets.push(&mutation.target);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecGrantRetryState {
    #[default]
    None,
    PendingApply,
    PendingDeny,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecGrantPersistResult {
    pub operation_id: String,
    pub target: crate::share::ExecGrantTarget,
    pub requested_enabled: bool,
    pub persisted: bool,
    pub applied: bool,
    pub revision: u64,
    pub retry_state: ExecGrantRetryState,
    pub error: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct StepProgress {
    persisted: bool,
    applied: bool,
    cleared: bool,
    error: Option<String>,
}

pub(super) fn load_pending() -> Result<Option<JournalEntry>, String> {
    let file = match crate::daemon::ipc_storage::open_exec_journal() {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Exec-Grant journal open: {error}")),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("Exec-Grant journal metadata: {error}"))?;
    if metadata.len() > MAX_JOURNAL_BYTES {
        return Err("Exec-Grant journal exceeds its 64 KiB limit".into());
    }
    let mut encoded = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|error| format!("Exec-Grant journal read: {error}"))?;
    if encoded.len() as u64 > MAX_JOURNAL_BYTES {
        return Err("Exec-Grant journal exceeds its 64 KiB limit".into());
    }
    let entry = decode_entry(&encoded)?;
    entry.validate()?;
    Ok(Some(entry))
}

pub(super) fn mask_pending(
    profiles: &mut crate::share::ShareProfiles,
    identity: &crate::share::ShareIdentity,
    entry: &JournalEntry,
) -> Result<(), String> {
    for (mutation, expected) in entry.steps() {
        mutation.mask_pending_policy(profiles, identity, expected)?;
    }
    Ok(())
}

pub(super) fn mask_all(profiles: &mut crate::share::ShareProfiles) {
    crate::share::ExecGrantMutation::mask_all_policies(profiles);
}

pub(super) fn prepare_pending_runtime(
    state: &mut ShareHostState,
    profiles: &mut crate::share::ShareProfiles,
    entry: &JournalEntry,
) -> Result<(), String> {
    let identity = state
        .identity
        .as_ref()
        .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())?;
    if let Err(error) = mask_pending(profiles, identity, entry) {
        mask_all(profiles);
        state.profiles = profiles.clone();
        state.profiles_error = Some(error.clone());
        stop_service_locked(state)?;
        return Err(format!("Exec-Grant Recovery verweigert: {error}"));
    }
    // The old runtime may already hold the desired revision from a previous
    // attempt. Recreate it from the masked profile instead of rolling back.
    stop_service_locked(state)
}

pub(super) fn recover_locked(
    state: &mut ShareHostState,
    entry: &JournalEntry,
) -> ExecGrantPersistResult {
    execute_locked(state, entry)
}

pub(super) fn recover_and_record(state: &mut ShareHostState, entry: &JournalEntry) {
    let result = recover_locked(state, entry);
    if let Some(error) = &result.error {
        super::ui_events::push(
            &mut state.ui_events,
            crate::share::ShareEvent::Error(format!(
                "Exec-Grant Recovery {}: {error}",
                result.operation_id
            )),
        );
    }
    state.exec_retry = (result.retry_state != ExecGrantRetryState::None).then_some(result);
}

impl ShareHost {
    pub fn mutate_exec_grant(
        &self,
        target: crate::share::ExecGrantTarget,
        enabled: bool,
        settings: Option<crate::share::ExecGrantSettings>,
    ) -> Result<ExecGrantPersistResult, String> {
        // Before anything is locked or written: a refused change leaves the
        // grant exactly as it is.
        if let Some(settings) = &settings {
            settings.validate_for_this_host()?;
        }
        self.mutate_grant_with(|profiles, identity| {
            crate::share::ExecGrantMutation::prepare_persisted(
                profiles,
                identity,
                target,
                enabled,
                settings.as_ref(),
                crate::share::core_now_secs(),
            )
        })
    }

    /// Journal, persistence and worker hand-off of one grant change;
    /// `prepare` makes the change from the current profiles and returns it
    /// with the revision it replaces.
    pub fn mutate_grant_with<F>(&self, prepare: F) -> Result<ExecGrantPersistResult, String>
    where
        F: FnOnce(
            &mut crate::share::ShareProfiles,
            &crate::share::ShareIdentity,
        ) -> Result<(crate::share::ExecGrantMutation, u64), String>,
    {
        let _exclusive = self
            .exec_grant_lock
            .lock()
            .map_err(|_| "Exec-Grant mutation lock is poisoned".to_string())?;
        self.reload_now_locked()?;
        if load_pending()?.is_some() {
            return self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?
                .exec_retry
                .clone()
                .ok_or_else(|| "an Exec-Grant recovery is already pending".to_string());
        }
        self.mutate_grant_locked(prepare)
    }

    /// `mutate_grant_with` for a caller that holds `exec_grant_lock`, has
    /// reloaded the profiles and found no open journal entry.
    fn mutate_grant_locked<F>(&self, prepare: F) -> Result<ExecGrantPersistResult, String>
    where
        F: FnOnce(
            &mut crate::share::ShareProfiles,
            &crate::share::ShareIdentity,
        ) -> Result<(crate::share::ExecGrantMutation, u64), String>,
    {
        let (identity, mut profiles) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            let identity = state
                .identity
                .clone()
                .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())?;
            (identity, state.profiles.clone())
        };
        let (mutation, expected_policy_revision) = prepare(&mut profiles, &identity)?;
        let entry = JournalEntry::new(mutation, expected_policy_revision)?;
        write_entry(&entry)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
        let result = execute_locked(&mut state, &entry);
        state.exec_retry =
            (result.retry_state != ExecGrantRetryState::None).then(|| result.clone());
        Ok(result)
    }
}

fn execute_locked(state: &mut ShareHostState, entry: &JournalEntry) -> ExecGrantPersistResult {
    let identity = match state.identity.clone() {
        Some(identity) => identity,
        None => return failed_result(entry, "Share identity is unavailable"),
    };
    let service = state.service.clone();
    let mut committed = None;
    // A grant that allows nothing reaches the worker before persistence,
    // one that allows anything after it. For a single grant this is the
    // phase order; a confirmation (W8) applies its revocations first.
    let apply = |grants: bool| -> Result<(), String> {
        for (mutation, _) in entry.steps() {
            if mutation.policy.grants_any() == grants {
                apply_to_worker(service.as_ref(), mutation)?;
            }
        }
        Ok(())
    };
    let mut progress = drive_steps(
        entry.phase,
        || {
            if entry.phase == JournalPhase::PendingApply {
                apply(false)?;
                rights::apply_narrowing_first(service.as_ref(), entry, &state.profiles, &identity)?;
            }
            let profiles =
                crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |profiles| {
                    rights::apply_persisted(entry, profiles, &identity)
                })?;
            committed = Some(profiles);
            Ok(())
        },
        || apply(entry.phase == JournalPhase::PendingApply),
        || clear_entry(&entry.operation_id),
    );

    if should_fail_closed(entry.phase, &progress) {
        // A completed enable whose acknowledgement could not be made durable
        // must not leave the live authorization active while it is reported as
        // pending. Keep periodic reloads behind the same explicit refresh
        // barrier used by a manual worker stop.
        state.suspended = true;
        if let Err(stop_error) = stop_service_locked(state) {
            let clear_error = progress
                .error
                .take()
                .unwrap_or_else(|| "Exec-Grant acknowledgement is not durable".into());
            progress.error = Some(format!(
                "{clear_error}; fail-closed Share worker stop: {stop_error}"
            ));
        }
    }

    if progress.cleared {
        if let Some(profiles) = committed {
            state.profiles = profiles;
        }
    } else if let Some(mut profiles) = committed {
        if mask_pending(&mut profiles, &identity, entry).is_err() {
            mask_all(&mut profiles);
        }
        state.profiles = profiles;
    } else if mask_pending(&mut state.profiles, &identity, entry).is_err() {
        mask_all(&mut state.profiles);
    }

    ExecGrantPersistResult {
        operation_id: entry.operation_id.clone(),
        target: entry.mutation.target.clone(),
        requested_enabled: entry.mutation.policy.enabled,
        persisted: progress.persisted,
        applied: progress.applied,
        revision: entry.mutation.policy.policy_revision,
        retry_state: if progress.cleared {
            ExecGrantRetryState::None
        } else {
            entry.phase.retry_state()
        },
        error: progress.error,
    }
}

fn apply_to_worker(
    service: Option<&crate::share::ShareService>,
    mutation: &crate::share::ExecGrantMutation,
) -> Result<(), String> {
    match service {
        Some(service) => {
            let applied = service.apply_persisted_exec_grant(
                mutation.target.clone(),
                mutation.principal.clone(),
                mutation.policy.clone(),
            )?;
            if applied.target != mutation.target
                || applied.principal != mutation.principal
                || applied.policy != mutation.policy
            {
                return Err("Share worker acknowledged a different Exec-Grant".into());
            }
            Ok(())
        }
        None if !mutation.policy.grants_any() => Ok(()),
        None => Err("Share worker is not active; apply remains pending".into()),
    }
}

fn should_fail_closed(phase: JournalPhase, progress: &StepProgress) -> bool {
    phase == JournalPhase::PendingApply && progress.applied && !progress.cleared
}

fn failed_result(entry: &JournalEntry, error: impl Into<String>) -> ExecGrantPersistResult {
    ExecGrantPersistResult {
        operation_id: entry.operation_id.clone(),
        target: entry.mutation.target.clone(),
        requested_enabled: entry.mutation.policy.enabled,
        persisted: false,
        applied: false,
        revision: entry.mutation.policy.policy_revision,
        retry_state: entry.phase.retry_state(),
        error: Some(error.into()),
    }
}

fn drive_steps<P, A, C>(
    phase: JournalPhase,
    mut persist: P,
    mut apply: A,
    mut clear: C,
) -> StepProgress
where
    P: FnMut() -> Result<(), String>,
    A: FnMut() -> Result<(), String>,
    C: FnMut() -> Result<(), String>,
{
    let mut progress = StepProgress::default();
    let ordered = match phase {
        JournalPhase::PendingApply => {
            if let Err(error) = persist() {
                progress.error = Some(error);
                return progress;
            }
            progress.persisted = true;
            apply().map(|_| progress.applied = true)
        }
        JournalPhase::PendingDeny => {
            if let Err(error) = apply() {
                progress.error = Some(error);
                return progress;
            }
            progress.applied = true;
            persist().map(|_| progress.persisted = true)
        }
    };
    if let Err(error) = ordered {
        progress.error = Some(error);
        return progress;
    }
    match clear() {
        Ok(()) => progress.cleared = true,
        Err(error) => progress.error = Some(error),
    }
    progress
}

#[cfg(test)]
#[path = "exec_grant_journal_tests.rs"]
mod tests;
