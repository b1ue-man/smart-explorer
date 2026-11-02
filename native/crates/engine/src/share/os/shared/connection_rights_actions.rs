//! Persisted F1–F6 actions (plan 2026-10-31): confirm or cancel the rights
//! of a connection, the rights of the Direct code and of a new room. Each is
//! one profile transaction; the caller refreshes the worker afterwards, which
//! applies the stored rights (persisted before they apply). W8 (review
//! 2026-10-31): a confirmation that changes commands, terminal or screen of
//! any grant goes as a whole through the background worker's Exec-grant
//! journal (`daemon/os/shared/exec_grant_journal_rights.rs`), like every
//! other change of those rights, so a revocation holds without a reload.
//! Review 2, follow-up: every other path that gives or changes those rights
//! (accepting a request with rights, the Direct code's rights, a PIN
//! offer's rights, the rights chosen for a room, the automatic acceptance)
//! stores everything but them in its own transaction and hands them to the
//! same journal (`exec_follow_up.rs`); `require_exec_journal` makes such a
//! decision fail as a whole before anything is stored when the journal
//! cannot be used, and `finish_exec_rights` completes it afterwards.
//! Review 3, W2/G3: „Abbrechen“ leaves a connection with open rights without
//! any right, also without the folders and write right a deferred acceptance
//! stored (`cancel_connection_rights`; commands, terminal and screen go
//! through the same journal); a waiting change that takes rights away is in
//! force at the worker at once (`exec_follow_up.rs`), and the message of a
//! failed journal step says „noch nicht entzogen“ for it
//! (`exec_open_text`), „noch nicht freigegeben“ for a change that gives.
use crate::share::access_grant::AccountProfile;
use crate::share::connection_rights::{
    ConnectionRights, RightsOrigin, RightsTarget, MENU_CAPABILITIES,
};
use crate::share::connection_rights_exec::changed_exec_grants;
use crate::share::exec_follow_up::{pins_complete, FollowUpEffect};
use crate::share::{DirectPeerIdentity, ShareProfiles, SharedRoot, DIRECT_EXPORT_SCOPE};

/// A grant's Exec right changed between the check and the transaction.
const CONCURRENT_EXEC: &str =
    "Befehle, Terminal oder Bildschirm wurden gleichzeitig geaendert; bitte erneut bestaetigen";

/// What one action did: the committed profiles and whether they changed.
pub struct RightsChange {
    pub profiles: ShareProfiles,
    pub changed: bool,
}

/// The host rules every surface checks before rights are stored, the same
/// as for an access change over IPC (`access_grant_journal.rs`): another
/// account than the worker's needs a worker that can switch to it.
pub fn validate_rights_for_this_host(rights: &ConnectionRights) -> Result<(), String> {
    rights.validate()?;
    let exec_on = rights.commands.is_on() || rights.terminal.is_on();
    if exec_on && rights.account != AccountProfile::Own && !crate::share::exec_run_as_supported() {
        return Err(
            "this host runs commands only as the Smart Explorer user (Standardnutzer); \
             another account needs a Linux worker running as root"
                .into(),
        );
    }
    Ok(())
}

fn mutate(
    default_home: Option<String>,
    mut action: impl FnMut(&mut ShareProfiles, i64) -> Result<bool, String>,
) -> Result<RightsChange, String> {
    let now = crate::share::core_now_secs();
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home, |profiles| {
        changed = action(profiles, now)?;
        Ok(())
    })?;
    Ok(RightsChange { profiles, changed })
}

/// The profile change of „Bestätigen“: the folders picked in the menu
/// (`added`, label and path) become Direct exports, then `target` gets
/// exactly `rights`. All or nothing on `profiles`; returns whether anything
/// changed. The daemon's journal runs the same function when it persists.
pub fn plan_confirm(
    profiles: &mut ShareProfiles,
    target: &RightsTarget,
    rights: &ConnectionRights,
    added: &[(String, String)],
    now: i64,
) -> Result<bool, String> {
    let mut candidate = profiles.clone();
    let mut changed = false;
    for (label, path) in added {
        let known = candidate
            .default_direct_exports
            .roots
            .iter()
            .any(|root| &root.path == path);
        if !known {
            candidate
                .default_direct_exports
                .roots
                .push(SharedRoot::new_in_scope(label, path, DIRECT_EXPORT_SCOPE));
            changed = true;
        }
    }
    changed |= candidate.confirm_connection_rights(target, rights, now)?;
    *profiles = candidate;
    Ok(changed)
}

/// „Bestätigen“: exactly `rights` for `target`, all or nothing.
pub fn confirm_connection_rights(
    default_home: Option<String>,
    target: &RightsTarget,
    rights: &ConnectionRights,
) -> Result<RightsChange, String> {
    confirm_connection_rights_with_folders(default_home, target, rights, &[])
}

/// „Bestätigen“ with the folders picked in the menu (`added`, label and
/// path), which become Direct exports in the same step. A change of
/// commands, terminal or screen goes as a whole through the worker's
/// Exec-grant journal (W8); the worker is started when needed, as for
/// every other change of those rights. Any other confirmation is one
/// profile transaction.
pub fn confirm_connection_rights_with_folders(
    default_home: Option<String>,
    target: &RightsTarget,
    rights: &ConnectionRights,
    added: &[(String, String)],
) -> Result<RightsChange, String> {
    validate_rights_for_this_host(rights)?;
    let current = ShareProfiles::load_checked(default_home.clone())?;
    let mut desired = current.clone();
    plan_confirm(
        &mut desired,
        target,
        rights,
        added,
        crate::share::core_now_secs(),
    )?;
    if changed_exec_grants(&current, &desired)?.is_empty() {
        return confirm_without_exec(default_home, target, rights, added);
    }
    let changed =
        crate::daemon::confirm_rights_in_worker(target.clone(), rights.clone(), added.to_vec())?;
    let profiles = ShareProfiles::load_checked(default_home)?;
    Ok(RightsChange { profiles, changed })
}

/// One profile transaction for a confirmation that changes no Exec grant.
/// Refused when one would change by the time it commits, so no change of
/// commands, terminal or screen bypasses the journal.
pub fn confirm_without_exec(
    default_home: Option<String>,
    target: &RightsTarget,
    rights: &ConnectionRights,
    added: &[(String, String)],
) -> Result<RightsChange, String> {
    mutate(default_home, |profiles, now| {
        let before = profiles.clone();
        let changed = plan_confirm(profiles, target, rights, added, now)?;
        if !changed_exec_grants(&before, profiles)?.is_empty() {
            return Err(CONCURRENT_EXEC.into());
        }
        Ok(changed)
    })
}

/// „Abbrechen“: the connection stays and has no right afterwards (W2,
/// review 3 of 2026-10-31), also when its folders and write right were
/// stored already and only commands, terminal and screen wait for the
/// Exec-grant journal (`exec_follow_up.rs`). Folders and write right are
/// taken back. When the connection holds commands, terminal or screen, or a
/// change of them waits, the worker's journal takes them back too
/// (`rights_after_cancel` confirmed through the worker: revocation at the
/// running worker before it is stored, started when needed, like every
/// other change of those rights). Without the journal (`cancel_without_journal`)
/// folders and write right still go, the revocation is recorded for the
/// background worker and the error says what is not taken back yet.
pub fn cancel_connection_rights(
    default_home: Option<String>,
    target: &RightsTarget,
) -> Result<RightsChange, String> {
    let current = ShareProfiles::load_checked(default_home.clone())?;
    if !current.cancel_needs_journal(target) {
        return cancel_in_profiles(default_home, target);
    }
    let rights = current.rights_after_cancel(target);
    match crate::daemon::confirm_rights_in_worker(target.clone(), rights, Vec::new()) {
        Ok(changed) => Ok(RightsChange {
            profiles: ShareProfiles::load_checked(default_home)?,
            changed,
        }),
        Err(error) => cancel_without_journal(default_home, target, &error),
    }
}

/// „Abbrechen“ as one profile transaction (nothing of commands, terminal or
/// screen is taken back here).
pub fn cancel_in_profiles(
    default_home: Option<String>,
    target: &RightsTarget,
) -> Result<RightsChange, String> {
    mutate(default_home, |profiles, now| {
        Ok(profiles.cancel_connection_rights(target, now))
    })
}

/// „Abbrechen“ when the journal could not be used (`error`): the profile
/// transaction takes folders and write right back and records the
/// revocation of commands, terminal and screen for the background worker.
/// It succeeds when nothing of them is held; else the connection stays
/// „Rechte offen“ and the error names what is not taken back yet.
pub fn cancel_without_journal(
    default_home: Option<String>,
    target: &RightsTarget,
    error: &str,
) -> Result<RightsChange, String> {
    let change = cancel_in_profiles(default_home, target)?;
    if change.profiles.pending_rights_of(target).is_none() {
        return Ok(change);
    }
    Err(format!(
        "Ordner und Schreiben sind entzogen; Befehle, Terminal oder Bildschirm noch nicht \
         entzogen (Rechte offen; der Hintergrunddienst entzieht sie beim naechsten Laden): {error}"
    ))
}

/// F5/F6: the rights of devices that connect with this Direct code.
pub fn set_code_rights(
    default_home: Option<String>,
    rights: Option<ConnectionRights>,
) -> Result<RightsChange, String> {
    if let Some(rights) = &rights {
        validate_rights_for_this_host(rights)?;
    }
    mutate(default_home, |profiles, _| {
        profiles.set_code_rights(rights.clone())
    })
}

/// After a room was created here: `rights` chosen in the same step (F5), or
/// the room waits for its rights as a created room (F1).
pub fn finish_room_creation(
    default_home: Option<String>,
    profile_id: &str,
    rights: Option<&ConnectionRights>,
) -> Result<RightsChange, String> {
    if let Some(rights) = rights {
        validate_rights_for_this_host(rights)?;
    }
    let target = RightsTarget::Room {
        profile_id: profile_id.into(),
    };
    mutate(default_home, |profiles, now| match rights {
        // The rights of members admitted already reach their grants through
        // the Exec-grant journal (`RoomPolicy::member_follow_ups`).
        Some(rights) => {
            profiles.apply_room_connection_rights_deferring_exec(profile_id, rights, now)
        }
        None => {
            profiles.cancel_connection_rights(&target, now);
            profiles.mark_room_rights_pending(profile_id, RightsOrigin::RoomCreated, now);
            Ok(true)
        }
    })
}

/// Start of the failure of `finish_exec_rights`: the connection (and its
/// folders and write right) was stored, only commands, terminal or screen
/// are still open.
pub const CONNECTION_STORED: &str = "Verbindung gespeichert, ";

/// The text a surface shows for a failed decision: a failure after the
/// connection was stored says so, any other one says the decision was not
/// stored (`not_stored`, the surface's own wording).
pub fn decision_failure(error: &str, not_stored: &str) -> String {
    if error.starts_with(CONNECTION_STORED) {
        error.to_string()
    } else {
        format!("{not_stored}: {error}")
    }
}

/// Whether `rights` touch commands, terminal or screen.
fn touches_exec(rights: &ConnectionRights) -> bool {
    MENU_CAPABILITIES
        .into_iter()
        .any(|capability| rights.decision(capability).is_on())
}

/// Before an acceptance of `requester` that may give or change commands,
/// terminal or screen of its grant: `rights` chosen with the acceptance, or
/// else the Direct code's rights (which a new connection takes). Those
/// rights reach the grant only through the Exec-grant journal of the
/// background worker, so the acceptance fails as a whole, before anything
/// is stored, when the journal cannot be used: the identity must be fully
/// pinned (`public_key` equal to `node_id`, matching fingerprint) and the
/// background worker of this version must be reachable (`ensure_worker_ready`
/// replaces an older one). Returns whether `finish_exec_rights` follows the
/// transaction. An acceptance that touches none of those rights needs
/// neither the worker nor the check.
pub fn require_exec_journal(
    profiles: &ShareProfiles,
    requester: &DirectPeerIdentity,
    rights: Option<&ConnectionRights>,
) -> Result<bool, String> {
    let existing_exec = profiles
        .grant_for(&requester.device_id)
        .is_some_and(|grant| grant.exec.grants_any());
    let chosen = rights.or(profiles.code_rights.as_ref());
    let involved = chosen.is_some_and(touches_exec) || (rights.is_some() && existing_exec);
    if !involved {
        return Ok(false);
    }
    if !pins_complete(
        &requester.public_key,
        &requester.node_id,
        &requester.fingerprint,
    ) {
        return Err(
            "Befehle, Terminal oder Bildschirm brauchen eine vollstaendig gepinnte Identitaet \
             (Schluessel gleich Knoten-ID, passender Fingerabdruck); die Anfrage bleibt unveraendert"
                .into(),
        );
    }
    crate::daemon::ensure_worker_ready().map_err(|error| {
        format!(
            "Befehle, Terminal oder Bildschirm gehen nur ueber das Exec-Grant-Journal des \
             Hintergrunddienstes; er ist nicht erreichbar, die Anfrage bleibt unveraendert: {error}"
        )
    })?;
    Ok(true)
}

/// After the transaction of an acceptance for which `require_exec_journal`
/// returned `true`: hands the commands, terminal and screen of the
/// requester to the Exec-grant journal when the transaction recorded them
/// (`exec_follow_ups`). The connection exists by then and waits without
/// those rights („Rechte offen“); a failure names the reason, and the
/// background worker repeats the change on its next reload.
pub fn finish_exec_rights(
    default_home: Option<String>,
    device_id: &str,
    rights: Option<&ConnectionRights>,
) -> Result<(), String> {
    let profiles = ShareProfiles::load_checked(default_home)?;
    let Some(grant) = profiles.grant_for(device_id) else {
        return Ok(());
    };
    let waiting = profiles.exec_follow_ups.iter().any(|follow_up| {
        follow_up.device_id == device_id && follow_up.public_key == grant.public_key
    });
    if !waiting {
        return Ok(());
    }
    let Some(rights) = rights.cloned().or_else(|| profiles.code_rights.clone()) else {
        return Ok(());
    };
    let effect = profiles.direct_follow_up_effect(device_id, crate::share::core_now_secs());
    let target = RightsTarget::Direct {
        device_id: grant.device_id.clone(),
        public_key: grant.public_key.clone(),
    };
    crate::daemon::confirm_rights_in_worker(target, rights, Vec::new())
        .map(|_| ())
        .map_err(|error| {
            format!(
                "{CONNECTION_STORED}Befehle, Terminal oder Bildschirm {}: {error}",
                exec_open_text(effect)
            )
        })
}

/// What the message of a failed journal step says is still open: a change
/// that takes rights away is „noch nicht entzogen“ (the worker holds it
/// masked until the journal stores it), one that gives rights is „noch nicht
/// freigegeben“.
pub fn exec_open_text(effect: FollowUpEffect) -> &'static str {
    match (effect.narrows, effect.widens) {
        (true, false) => {
            "noch nicht entzogen (Rechte offen; der Hintergrunddienst sperrt sie beim \
             naechsten Laden und wiederholt den Entzug)"
        }
        (true, true) => {
            "zum Teil noch nicht entzogen, zum Teil noch nicht freigegeben (Rechte offen; \
             der Hintergrunddienst sperrt den Entzug beim naechsten Laden)"
        }
        _ => "noch nicht freigegeben (Rechte offen)",
    }
}

#[cfg(test)]
#[path = "abgleich_task_exec_journal_gate_tests.rs"]
mod abgleich_task_tests;
