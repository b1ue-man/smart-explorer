//! Persisted F1–F6 actions (plan 2026-10-31): confirm or cancel the rights
//! of a connection, the rights of the Direct code and of a new room. Each is
//! one profile transaction; the caller refreshes the worker afterwards, which
//! applies the stored rights (persisted before they apply). W8 (review
//! 2026-10-31): a confirmation that changes commands, terminal or screen of
//! any grant goes as a whole through the background worker's Exec-grant
//! journal (`daemon/os/shared/exec_grant_journal_rights.rs`), like every
//! other change of those rights, so a revocation holds without a reload.
use crate::share::access_grant::AccountProfile;
use crate::share::connection_rights::{ConnectionRights, RightsOrigin, RightsTarget};
use crate::share::connection_rights_exec::changed_exec_grants;
use crate::share::{ShareProfiles, SharedRoot, DIRECT_EXPORT_SCOPE};

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
pub(crate) fn plan_confirm(
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
pub(crate) fn confirm_without_exec(
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

/// „Abbrechen“: the connection stays without rights.
pub fn cancel_connection_rights(
    default_home: Option<String>,
    target: &RightsTarget,
) -> Result<RightsChange, String> {
    mutate(default_home, |profiles, _| {
        Ok(profiles.cancel_connection_rights(target))
    })
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
        Some(rights) => profiles.apply_room_connection_rights(profile_id, rights, now),
        None => {
            profiles.cancel_connection_rights(&target);
            profiles.mark_room_rights_pending(profile_id, RightsOrigin::RoomCreated, now);
            Ok(true)
        }
    })
}
