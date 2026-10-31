//! W8 (review 2026-10-31): „Bestätigen“ of the compact rights menu through
//! the Exec-grant journal. A confirmation that changes commands, terminal
//! or screen of any grant (one Direct device, or every admitted member of a
//! room) is one journal entry (version 3): the grants and the rest of the
//! rights (folders, write, the Direct exports picked in the menu, the end of
//! „Rechte offen“) are persisted in one profile transaction, revocations
//! reach the running worker before that and grants after it, and a failure
//! leaves the entry pending: every reload masks its grants off and repeats
//! it, as for `MutateGrant`. It runs under the same lock as every other
//! grant change. A confirmation that changes no Exec grant stays one
//! profile transaction under that lock.
use serde::{Deserialize, Serialize};

use super::{
    default_home, execute_locked, load_pending, write_entry, ExecGrantRetryState, JournalEntry,
    JournalPhase, ShareHost, CONFIRM_JOURNAL_VERSION,
};
use crate::share::connection_rights::{ConnectionRights, RightsTarget};
use crate::share::connection_rights_actions::{self as actions, RightsChange};
use crate::share::connection_rights_exec::changed_exec_grants;
use crate::share::{ExecGrantMutation, ShareIdentity, ShareProfiles};

/// One further grant of a confirmation with the revision it replaces.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct GrantStep {
    pub(super) mutation: ExecGrantMutation,
    pub(super) expected_policy_revision: u64,
}

/// The confirmation an entry carries: exactly `rights` for `target`, with
/// the folders picked in the menu (`added`, label and path).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct RightsStep {
    pub(super) target: RightsTarget,
    pub(super) rights: ConnectionRights,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) added: Vec<(String, String)>,
    /// Unix seconds of the confirmation (the grants' `changed_at`).
    pub(super) changed_at: i64,
}

/// The grants a confirmation changes from `before` to `after`, each as the
/// journal step that sets exactly its new policy.
pub(super) fn grant_steps(
    before: &ShareProfiles,
    after: &ShareProfiles,
    identity: &ShareIdentity,
) -> Result<Vec<GrantStep>, String> {
    let mut scratch = before.clone();
    changed_exec_grants(before, after)?
        .into_iter()
        .map(|(target, policy)| {
            let (mutation, expected_policy_revision) =
                ExecGrantMutation::prepare_exact(&mut scratch, identity, target, policy)?;
            Ok(GrantStep {
                mutation,
                expected_policy_revision,
            })
        })
        .collect()
}

/// The entry of a confirmation; `None` when it changes no grant.
pub(super) fn confirm_entry(
    mut steps: Vec<GrantStep>,
    rights: RightsStep,
) -> Result<Option<JournalEntry>, String> {
    if steps.is_empty() {
        return Ok(None);
    }
    let first = steps.remove(0);
    let mut entry = JournalEntry::new(first.mutation, first.expected_policy_revision)?;
    entry.version = CONFIRM_JOURNAL_VERSION;
    entry.more = steps;
    entry.rights = Some(rights);
    entry.phase = if entry
        .steps()
        .any(|(mutation, _)| mutation.policy.grants_any())
    {
        JournalPhase::PendingApply
    } else {
        JournalPhase::PendingDeny
    };
    entry.validate()?;
    Ok(Some(entry))
}

/// The persistence step of `entry` on the stored profiles: every grant
/// (compare-and-set against the revision it replaces, ending the follow-up
/// record that waited for it), then the rest of its confirmation. Repeating it after a crash changes nothing more; a
/// confirmation that would by now change another grant is refused.
pub(super) fn apply_persisted(
    entry: &JournalEntry,
    profiles: &mut ShareProfiles,
    identity: &ShareIdentity,
) -> Result<(), String> {
    for (mutation, expected) in entry.steps() {
        mutation.apply_persisted_cas(profiles, identity, expected)?;
        // A change of the grant that waited for this step is stored with
        // it, and an explicit change supersedes a waiting one
        // (`exec_follow_up.rs`).
        profiles.finish_exec_follow_ups(&mutation.target);
    }
    let Some(step) = &entry.rights else {
        return Ok(());
    };
    let before = profiles.clone();
    actions::plan_confirm(
        profiles,
        &step.target,
        &step.rights,
        &step.added,
        step.changed_at,
    )?;
    if !changed_exec_grants(&before, profiles)?.is_empty() {
        return Err(
            "Befehle, Terminal oder Bildschirm wurden gleichzeitig geaendert; bitte erneut bestaetigen"
                .into(),
        );
    }
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "ja"
    } else {
        "nein"
    }
}

impl ShareHost {
    /// „Bestätigen“ of the daemon: exactly `rights` for `target`, all or
    /// nothing, with `added` (label, path) as new Direct exports. The
    /// caller reloads the worker afterwards for folders and write.
    pub(crate) fn confirm_rights_journaled(
        &self,
        target: &RightsTarget,
        rights: &ConnectionRights,
        added: &[(String, String)],
    ) -> Result<RightsChange, String> {
        actions::validate_rights_for_this_host(rights)?;
        let _exclusive = self
            .exec_grant_lock
            .lock()
            .map_err(|_| "Exec-Grant mutation lock is poisoned".to_string())?;
        self.reload_now_locked()?;
        if load_pending()?.is_some() {
            return Err(
                "Eine fruehere Aenderung von Befehlen, Terminal oder Bildschirm wird noch \
                        wiederholt; bitte danach erneut bestaetigen"
                    .into(),
            );
        }
        let (identity, profiles) = {
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
        let changed_at = crate::share::core_now_secs();
        let mut desired = profiles.clone();
        actions::plan_confirm(&mut desired, target, rights, added, changed_at)?;
        let steps = grant_steps(&profiles, &desired, &identity)?;
        let step = RightsStep {
            target: target.clone(),
            rights: rights.clone(),
            added: added.to_vec(),
            changed_at,
        };
        let Some(entry) = confirm_entry(steps, step)? else {
            return actions::confirm_without_exec(Some(default_home()), target, rights, added);
        };
        write_entry(&entry)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
        let result = execute_locked(&mut state, &entry);
        state.exec_retry =
            (result.retry_state != ExecGrantRetryState::None).then(|| result.clone());
        if let Some(error) = result.error {
            return Err(format!(
                "Rechte nicht vollstaendig uebernommen (gespeichert: {}, angewandt: {}); \
                 der Hintergrunddienst wiederholt die Bestaetigung als Ganzes: {error}",
                yes_no(result.persisted),
                yes_no(result.applied)
            ));
        }
        Ok(RightsChange {
            profiles: state.profiles.clone(),
            changed: true,
        })
    }
}

#[cfg(test)]
#[path = "abgleich_task_confirm_journal_tests.rs"]
mod abgleich_task_tests;
