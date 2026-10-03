//! Relation updates from verified worker events: presences, offline notices,
//! legacy decisions, room rosters and LAN sightings. Pinned identity values are
//! never replaced (S29, S30), room members are keyed by their key (S22) and
//! wait for admission after a block (B15). Whether the worker then needs a new
//! configuration or only runtime data (FA3) is decided by the caller from the
//! committed profiles.
use crate::share::{
    DirectAccessState, DirectRequestDirection, PeerPresence, ShareEvent, ShareProfiles, ShareStatus,
};

/// Result of one worker event for the relations.
pub(super) enum RelationEvent {
    /// Not a relation event; the caller handles it.
    Other,
    Applied {
        /// The profiles changed and must be committed.
        changed: bool,
        /// The event goes on to the UI backlog.
        forward: bool,
        /// A problem to show instead.
        error: Option<String>,
    },
}

impl RelationEvent {
    fn changed(changed: bool) -> Self {
        Self::Applied {
            changed,
            forward: true,
            error: None,
        }
    }

    /// Room events need the local device id; without it they are dropped.
    fn dropped() -> Self {
        Self::Applied {
            changed: false,
            forward: false,
            error: None,
        }
    }
}

pub(super) fn apply(
    profiles: &mut ShareProfiles,
    local_device_id: Option<&str>,
    event: &ShareEvent,
    now: i64,
) -> RelationEvent {
    match event {
        ShareEvent::DirectAvailable {
            lookup_id,
            presence,
        } => RelationEvent::changed(
            profiles
                .direct_contacts
                .iter_mut()
                .find(|contact| contact.lookup_id == *lookup_id)
                .map(|contact| contact.apply_verified_presence(presence.clone(), now))
                .is_some(),
        ),
        ShareEvent::DirectOffline { lookup_id } => {
            let contact = profiles
                .direct_contacts
                .iter_mut()
                .find(|contact| contact.lookup_id == *lookup_id);
            RelationEvent::changed(contact.is_some_and(|contact| {
                contact.status = ShareStatus::Offline;
                contact.presence = None;
                true
            }))
        }
        ShareEvent::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            ..
        } => {
            let Some(local_device_id) = local_device_id else {
                return RelationEvent::Applied {
                    changed: false,
                    forward: false,
                    error: Some("Share-Identitaet ist nicht verfuegbar".into()),
                };
            };
            if requester_device_id != local_device_id {
                return RelationEvent::dropped();
            }
            RelationEvent::changed(apply_legacy_decision(
                profiles,
                lookup_id,
                *accepted,
                presence.clone(),
                now,
            ))
        }
        ShareEvent::RoomRoster { room_id, members } => {
            let Some(local_device_id) = local_device_id else {
                return RelationEvent::dropped();
            };
            let Some(room) = profiles
                .rooms
                .iter_mut()
                .find(|room| room.room_id == *room_id)
            else {
                return RelationEvent::changed(false);
            };
            room.status = ShareStatus::Available;
            room.last_seen = Some(now);
            for presence in members {
                if presence.device_id != local_device_id {
                    room.upsert_member_from_presence(presence.clone(), now);
                }
            }
            RelationEvent::changed(true)
        }
        ShareEvent::RoomJoined { room_id, presence } => {
            let Some(local_device_id) = local_device_id else {
                return RelationEvent::dropped();
            };
            let changed = presence.device_id != local_device_id
                && profiles
                    .rooms
                    .iter_mut()
                    .find(|room| room.room_id == *room_id)
                    .map(|room| room.upsert_member_from_presence(presence.clone(), now))
                    .is_some();
            RelationEvent::changed(changed)
        }
        ShareEvent::RoomLeft { room_id, device_id } => {
            let member = profiles
                .rooms
                .iter_mut()
                .find(|room| room.room_id == *room_id)
                .and_then(|room| {
                    room.members
                        .iter_mut()
                        .find(|member| member.device_id == *device_id)
                });
            RelationEvent::changed(member.is_some_and(|member| {
                member.status = ShareStatus::Offline;
                member.relay_url.clear();
                member.candidates.clear();
                member.presence = None;
                true
            }))
        }
        ShareEvent::LanPeerSeen {
            contact_id,
            candidates,
            uplink,
        } => {
            let contact = profiles
                .direct_contacts
                .iter_mut()
                .find(|contact| contact.id == *contact_id);
            RelationEvent::changed(contact.is_some_and(|contact| {
                contact.lan_candidates = candidates.clone();
                contact.lan_seen_at = Some(now);
                contact.lan_uplink = Some(*uplink);
                contact.last_seen = Some(now);
                if matches!(contact.status, ShareStatus::Offline | ShareStatus::Waiting)
                    && contact.access_state == DirectAccessState::Accepted
                {
                    contact.status = ShareStatus::Available;
                }
                true
            }))
        }
        ShareEvent::LanPeerLost { contact_id } => {
            let contact = profiles
                .direct_contacts
                .iter_mut()
                .find(|contact| contact.id == *contact_id);
            RelationEvent::changed(contact.is_some_and(|contact| {
                contact.lan_candidates.clear();
                contact.lan_seen_at = None;
                contact.lan_uplink = None;
                let server_presence = contact
                    .presence
                    .as_ref()
                    .is_some_and(|presence| presence.is_current_at(now));
                if !server_presence && contact.status == ShareStatus::Available {
                    contact.status = ShareStatus::Offline;
                }
                true
            }))
        }
        _ => RelationEvent::Other,
    }
}

/// A legacy answer only settles a request that went over the legacy path and
/// still waits; it never overrides a tracked (signed) decision or a settled
/// relation, and its free text is not trusted (S30, S46).
fn apply_legacy_decision(
    profiles: &mut ShareProfiles,
    lookup_id: &str,
    accepted: bool,
    presence: Option<PeerPresence>,
    now: i64,
) -> bool {
    let tracked_contacts: Vec<String> = profiles
        .direct_requests
        .iter()
        .filter(|entry| entry.direction == DirectRequestDirection::Outgoing)
        .filter_map(|entry| entry.contact_id.clone())
        .collect();
    let Some(contact) = profiles
        .direct_contacts
        .iter_mut()
        .find(|contact| contact.lookup_id == lookup_id)
    else {
        return false;
    };
    if contact.access_state != DirectAccessState::Pending || tracked_contacts.contains(&contact.id)
    {
        return false;
    }
    if !accepted {
        contact.access_state = DirectAccessState::Ignored;
        contact.status = ShareStatus::Failed("Freigabe abgelehnt".into());
        return true;
    }
    contact.access_state = DirectAccessState::Accepted;
    if let Some(presence) = presence {
        if contact.apply_verified_presence(presence, now) == crate::share::PresenceApply::Conflict {
            contact.access_state = DirectAccessState::Pending;
            return true;
        }
    }
    contact.accepted_at = Some(now);
    contact.accepted_public_key = contact.remote_public_key.clone();
    contact.status = ShareStatus::Available;
    contact.last_error = None;
    true
}
