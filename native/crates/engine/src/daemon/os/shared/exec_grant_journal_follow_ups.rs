//! Review 2, follow-up of W8: the daemon applies the changes of commands,
//! terminal and screen that profile transactions outside the Exec-grant
//! journal could only record (`share/core/exec_follow_up.rs`): the rights
//! chosen with an acceptance, with the Direct code, with a PIN offer or
//! with a room, and the automatic acceptance. Each record becomes one
//! ordinary single-grant journal entry under `exec_grant_lock`: a
//! revocation reaches the running worker before it is stored, a grant after
//! it, a crash between the steps leaves the entry open (every reload masks
//! the grant and repeats it), and storing the grant ends the record in the
//! same profile transaction (`exec_grant_journal_rights.rs`).
//!
//! A waiting change that takes rights away is in force at the running worker
//! at once, before the journal stores it (G3, review 3 of 2026-10-31): every
//! start and configuration of the worker masks those grants off
//! (`ipc_host_service.rs::worker_profiles`), as the recovery of a pending
//! journal entry masks its grants. The stored profiles and the profiles of
//! the host state stay as they are: the journal derives its entries from
//! them.
//!
//! A record that can never be applied (the grant is gone, not active or
//! without a fully pinned identity) is dropped with a visible error. Every
//! other failure keeps the record; the reload tries again after a backoff
//! that starts at `RETRY_AFTER` and doubles up to `LONGEST_RETRY`, and at
//! once when the profiles change beyond runtime data (A11 of plan
//! 2026-11-06-akku-rechte): each attempt is a complete reload.
use std::time::{Duration, Instant};

use super::load_pending;
use crate::share::access_grant::{AccessGrantChange, AccountProfile, Capability};
use crate::share::exec_follow_up::{FollowUpKey, FollowUpWork};
use crate::share::{ExecGrantMutation, ShareEvent, ShareProfiles};

/// How long a failed record waits before the second attempt.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// Longest wait. The failures that remain after the final ones are dropped
/// are a pending journal entry (its own retry stores the profiles, which
/// retries this at once) and storage errors of the profiles or the journal;
/// a profile change retries at once, so the bound only paces a storage
/// error that persists without any change.
const LONGEST_RETRY: Duration = Duration::from_secs(30 * 60);

/// The wait after `failures` earlier failed attempts in a row.
fn retry_after(failures: u32) -> Duration {
    let factor = 1u32.checked_shl(failures).unwrap_or(u32::MAX);
    RETRY_AFTER.saturating_mul(factor).min(LONGEST_RETRY)
}

/// Why this host can never apply `change`, as the rights menu checks it
/// when the change is chosen (`validate_rights_for_this_host`).
fn host_refusal(change: &AccessGrantChange) -> Option<&'static str> {
    let other_account = change
        .account
        .as_ref()
        .is_some_and(|account| *account != AccountProfile::Own);
    if other_account && !crate::share::exec_run_as_supported() {
        return Some(
            "this host runs commands only as the Smart Explorer user (Standardnutzer); \
             another account needs a Linux worker running as root",
        );
    }
    change
        .capabilities
        .get(&Capability::LoginAccess)
        .is_some_and(|decision| decision.is_on())
        .then_some("login_access must be granted through the protected machine policy")
}

impl super::ShareHost {
    /// Applies every waiting change through the journal, one entry each.
    /// The caller holds `exec_grant_lock` and has reloaded the profiles.
    /// Returns how many were applied; an error leaves the rest waiting.
    pub fn drain_exec_follow_ups_locked(&self) -> Result<usize, String> {
        let mut applied = 0;
        let mut last = None;
        loop {
            if load_pending()?.is_some() {
                return Err(
                    "Eine fruehere Aenderung von Befehlen, Terminal oder Bildschirm wird noch \
                     wiederholt; die wartenden Rechte folgen danach"
                        .into(),
                );
            }
            let next = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?
                .profiles
                .next_exec_follow_up();
            let Some(next) = next else {
                return Ok(applied);
            };
            if last.as_ref() == Some(&next.key) {
                return Err("Wartende Rechte wurden vom Journal nicht beendet".into());
            }
            last = Some(next.key.clone());
            match next.work {
                FollowUpWork::Obsolete(reason) => self.drop_follow_up(&next.key, &reason)?,
                FollowUpWork::Apply { target, change } => {
                    if let Some(reason) = host_refusal(&change) {
                        self.drop_follow_up(&next.key, reason)?;
                        continue;
                    }
                    let now = crate::share::core_now_secs();
                    let result = self.mutate_grant_locked(|profiles, identity| {
                        ExecGrantMutation::prepare_access(
                            profiles, identity, target, &change, None, now,
                        )
                    })?;
                    if let Some(error) = result.error {
                        return Err(format!(
                            "Wartende Rechte nicht vollstaendig uebernommen (gespeichert: {}, \
                             angewandt: {}); der Hintergrunddienst wiederholt sie: {error}",
                            if result.persisted { "ja" } else { "nein" },
                            if result.applied { "ja" } else { "nein" },
                        ));
                    }
                    applied += 1;
                }
            }
        }
    }

    /// Drops one record that can never be applied and says why.
    fn drop_follow_up(&self, key: &FollowUpKey, reason: &str) -> Result<(), String> {
        let committed = ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
            profiles.discard_exec_follow_up(key);
            Ok(())
        })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
        state.profiles = committed;
        super::super::ui_events::push(
            &mut state.ui_events,
            ShareEvent::Error(format!(
                "Gewaehlte Rechte (Befehle, Terminal oder Bildschirm) verworfen: {reason}"
            )),
        );
        Ok(())
    }

    /// Whether a waiting change is due for an attempt: the periodic reload
    /// calls `reload_now`, which applies it.
    pub fn exec_follow_ups_due(&self) -> bool {
        let Ok(mut guard) = self.state.lock() else {
            return false;
        };
        let state = &mut *guard;
        if !state.profiles.exec_follow_ups_waiting() {
            state.reload_gate.follow_up.settled();
            return false;
        }
        if state.service.is_none() {
            return false;
        }
        if state
            .reload_gate
            .follow_up
            .profiles_changed(&state.profiles)
        {
            state.follow_up_retry_at = None;
        }
        state
            .follow_up_retry_at
            .is_none_or(|at| Instant::now() >= at)
    }

    /// The attempt of every reload (`reload_now`, with `exec_grant_lock`
    /// held): nothing to do without a waiting change or a running worker;
    /// a failure is shown once per reason and tried again after its
    /// backoff (`retry_after`). It never fails the reload.
    pub fn drain_exec_follow_ups_quietly(&self) {
        if !self.exec_follow_ups_due() {
            return;
        }
        let outcome = self.drain_exec_follow_ups_locked();
        let Ok(mut guard) = self.state.lock() else {
            return;
        };
        let state = &mut *guard;
        match outcome {
            Ok(_) => {
                state.follow_up_retry_at = None;
                state.follow_up_error = None;
                state.reload_gate.follow_up.settled();
            }
            Err(error) => {
                let failures = state.reload_gate.follow_up.failed(&state.profiles);
                state.follow_up_retry_at = Some(Instant::now() + retry_after(failures));
                if state.follow_up_error.as_deref() != Some(error.as_str()) {
                    super::super::ui_events::push(
                        &mut state.ui_events,
                        ShareEvent::Error(error.clone()),
                    );
                    state.follow_up_error = Some(error);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "abgleich_task_follow_up_journal_tests.rs"]
mod abgleich_task_tests;
