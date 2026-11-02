//! W8 (review 2026-10-31): which Exec grants a confirmation of connection
//! rights changes. „Bestätigen“ hands exactly these to the Exec-grant
//! journal of the daemon, the same path as every other change of commands,
//! terminal and screen; a confirmation that changes none of them stays one
//! profile transaction.
use crate::share::exec_policy::ExecGrant;
use crate::share::{ExecGrantTarget, ShareProfiles};

/// Every Direct grant and room member whose Exec grant differs between
/// `before` and `after`, with its exact target and its policy in `after`.
/// A grant or member that `after` adds or whose identity it changes is an
/// error: a confirmation only changes the rights of known identities.
pub fn changed_exec_grants(
    before: &ShareProfiles,
    after: &ShareProfiles,
) -> Result<Vec<(ExecGrantTarget, ExecGrant)>, String> {
    let mut changed = Vec::new();
    for grant in &after.direct_grants {
        let old = before
            .direct_grants
            .iter()
            .find(|old| old.device_id == grant.device_id);
        let Some(old) = old else {
            if grant.exec.grants_any() {
                return Err(format!(
                    "Neue Direkt-Freigabe {} mit Befehlen oder Bildschirm",
                    grant.device_id
                ));
            }
            continue;
        };
        if old.exec == grant.exec {
            continue;
        }
        if old.public_key != grant.public_key
            || old.fingerprint != grant.fingerprint
            || old.node_id != grant.node_id
        {
            return Err(
                "Die Identitaet der Direkt-Freigabe wurde geaendert; bitte neu laden".into(),
            );
        }
        changed.push((
            ExecGrantTarget::Direct {
                device_id: grant.device_id.clone(),
                public_key: grant.public_key.clone(),
                fingerprint: grant.fingerprint.clone(),
                node_id: grant.node_id.clone(),
            },
            grant.exec.clone(),
        ));
    }
    for room in &after.rooms {
        let old_room = before.rooms.iter().find(|old| old.id == room.id);
        for member in &room.members {
            let old = old_room.and_then(|old_room| {
                old_room
                    .members
                    .iter()
                    .find(|old| old.device_id == member.device_id)
            });
            let Some(old) = old else {
                if member.exec.grants_any() {
                    return Err(format!(
                        "Neues Raummitglied {} mit Befehlen oder Bildschirm",
                        member.device_id
                    ));
                }
                continue;
            };
            if old.exec == member.exec {
                continue;
            }
            if old.public_key != member.public_key
                || old.fingerprint != member.fingerprint
                || old.node_id != member.node_id
                || old_room.is_some_and(|old_room| old_room.room_id != room.room_id)
            {
                return Err(
                    "Die Identitaet eines Raummitglieds wurde geaendert; bitte neu laden".into(),
                );
            }
            changed.push((
                ExecGrantTarget::RoomMember {
                    room_id: room.room_id.clone(),
                    device_id: member.device_id.clone(),
                    public_key: member.public_key.clone(),
                    fingerprint: member.fingerprint.clone(),
                    node_id: member.node_id.clone(),
                },
                member.exec.clone(),
            ));
        }
    }
    Ok(changed)
}

#[cfg(test)]
#[path = "abgleich_task_rights_review_tests.rs"]
mod abgleich_task_tests;
