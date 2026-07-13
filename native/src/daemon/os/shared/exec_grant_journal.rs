use std::io::{self, Read};

use serde::{Deserialize, Serialize};

use super::{default_home, stop_service_locked, ShareHost, ShareHostState};

#[path = "exec_grant_journal_storage.rs"]
mod storage;

use storage::{clear_entry, write_entry};

const JOURNAL_VERSION: u32 = 2;
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
        let phase = if mutation.policy.enabled {
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
        };
        entry.validate()?;
        Ok(entry)
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != JOURNAL_VERSION {
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
        let phase_enabled = self.phase == JournalPhase::PendingApply;
        if self.mutation.policy.enabled != phase_enabled {
            return Err("Exec-Grant journal phase contradicts desired policy".into());
        }
        self.mutation
            .validate_persisted_shape(self.expected_policy_revision)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecGrantRetryState {
    #[default]
    None,
    PendingApply,
    PendingDeny,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExecGrantPersistResult {
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

fn decode_entry(encoded: &[u8]) -> Result<JournalEntry, String> {
    let mut value: serde_json::Value = serde_json::from_slice(encoded)
        .map_err(|error| format!("Exec-Grant journal decode: {error}"))?;
    if value.get("version").and_then(serde_json::Value::as_u64) == Some(1) {
        migrate_v1_target(&mut value)?;
    }
    serde_json::from_value(value).map_err(|error| format!("Exec-Grant journal decode: {error}"))
}

fn migrate_v1_target(value: &mut serde_json::Value) -> Result<(), String> {
    let principal = value
        .pointer("/mutation/principal")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "legacy Exec-Grant journal has no exact principal".to_string())?;
    let field = |name: &str| {
        principal
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("legacy Exec-Grant journal principal lacks {name}"))
    };
    let relation_kind = field("relation_kind")?;
    let relation_id = field("relation_id")?;
    let pins = serde_json::json!({
        "device_id": field("device_id")?,
        "public_key": field("public_key")?,
        "fingerprint": field("fingerprint")?,
        "node_id": field("node_id")?,
    });
    let target = match relation_kind.as_str() {
        "direct" => serde_json::json!({ "Direct": pins }),
        "room" => {
            let mut fields = pins
                .as_object()
                .cloned()
                .ok_or_else(|| "legacy Exec-Grant journal pins are invalid".to_string())?;
            fields.insert("room_id".into(), serde_json::Value::String(relation_id));
            serde_json::json!({ "RoomMember": fields })
        }
        _ => return Err("legacy Exec-Grant journal relation is unsupported".into()),
    };
    value["mutation"]["target"] = target;
    value["version"] = serde_json::json!(JOURNAL_VERSION);
    Ok(())
}

pub(super) fn mask_pending(
    profiles: &mut crate::share::ShareProfiles,
    identity: &crate::share::ShareIdentity,
    entry: &JournalEntry,
) -> Result<(), String> {
    entry
        .mutation
        .mask_pending_policy(profiles, identity, entry.expected_policy_revision)
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
    pub(crate) fn mutate_exec_grant(
        &self,
        target: crate::share::ExecGrantTarget,
        enabled: bool,
    ) -> Result<ExecGrantPersistResult, String> {
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
        let (mutation, expected_policy_revision) =
            crate::share::ExecGrantMutation::prepare_persisted(
                &mut profiles,
                &identity,
                target,
                enabled,
                crate::share::core_now_secs(),
            )?;
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
    let progress = drive_steps(
        entry.phase,
        || {
            let profiles =
                crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |profiles| {
                    entry.mutation.apply_persisted_cas(
                        profiles,
                        &identity,
                        entry.expected_policy_revision,
                    )
                })?;
            committed = Some(profiles);
            Ok(())
        },
        || match &service {
            Some(service) => {
                let applied = service.apply_persisted_exec_grant(
                    entry.mutation.target.clone(),
                    entry.mutation.principal.clone(),
                    entry.mutation.policy.clone(),
                )?;
                if applied.target != entry.mutation.target
                    || applied.principal != entry.mutation.principal
                    || applied.policy != entry.mutation.policy
                {
                    return Err("Share worker acknowledged a different Exec-Grant".into());
                }
                Ok(())
            }
            None if entry.phase == JournalPhase::PendingDeny => Ok(()),
            None => Err("Share worker is not active; apply remains pending".into()),
        },
        || clear_entry(&entry.operation_id),
    );

    let mut progress = progress;
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
