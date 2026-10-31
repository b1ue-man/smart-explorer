//! Persisted F1–F6 actions (plan 2026-10-31): confirm or cancel the rights
//! of a connection, the rights of the Direct code and of a new room. Each is
//! one profile transaction; the caller refreshes the worker afterwards, which
//! applies the stored rights (persisted before they apply).
use crate::share::access_grant::AccountProfile;
use crate::share::connection_rights::{ConnectionRights, RightsOrigin, RightsTarget};
use crate::share::ShareProfiles;

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

/// „Bestätigen“: exactly `rights` for `target`, in one transaction.
pub fn confirm_connection_rights(
    default_home: Option<String>,
    target: &RightsTarget,
    rights: &ConnectionRights,
) -> Result<RightsChange, String> {
    validate_rights_for_this_host(rights)?;
    mutate(default_home, |profiles, now| {
        profiles.confirm_connection_rights(target, rights, now)
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
